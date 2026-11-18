//! macOS : accès direct au périphérique via CoreAudio HAL.
//!
//! Séquence (même principe que `ao_coreaudio_exclusive` de mpv) :
//!   1. périphérique de sortie par défaut -> premier stream de sortie
//!   2. « hog mode » (kAudioDevicePropertyHogMode = notre PID) : le mixeur coreaudiod
//!      n'est plus dans le chemin et le DAC est à nous
//!   3. choix d'un *physical format* entier entrelacé à la fréquence EXACTE de la source,
//!      application, puis attente de la prise d'effet (le changement est asynchrone)
//!   4. IOProc temps réel : ring -> quantification -> buffer du driver
//! À la fermeture : arrêt, restauration du format physique d'origine, libération du hog mode.
//!
//! NB : non compilé dans l'environnement de rédaction (Linux). À valider sur Mac avec un vrai DAC.

use crate::{AudioError, ExclusiveOutput, Opened};
use audio_core::{integer_multiples, render::RenderCtx, ring::Ring, DeviceFormat, SampleKind, Shared, StreamFormat};
use coreaudio_sys::*;
use std::cell::UnsafeCell;
use std::ffi::c_void;
use std::sync::Arc;
use std::time::{Duration, Instant};
use std::{mem, ptr};

const MAIN: u32 = 0; // kAudioObjectPropertyElementMain
const GLOBAL: u32 = kAudioObjectPropertyScopeGlobal;
const OUTPUT: u32 = kAudioObjectPropertyScopeOutput;

fn addr(selector: u32, scope: u32) -> AudioObjectPropertyAddress {
    AudioObjectPropertyAddress { mSelector: selector, mScope: scope, mElement: MAIN }
}

fn check(st: OSStatus, what: &str) -> Result<(), AudioError> {
    if st == 0 {
        Ok(())
    } else {
        Err(AudioError::Os(format!("{what}: OSStatus {st}")))
    }
}

unsafe fn get<T: Copy>(obj: AudioObjectID, a: AudioObjectPropertyAddress, out: &mut T, what: &str) -> Result<(), AudioError> {
    let mut size = mem::size_of::<T>() as u32;
    check(AudioObjectGetPropertyData(obj, &a, 0, ptr::null(), &mut size, out as *mut T as *mut c_void), what)
}

unsafe fn set<T>(obj: AudioObjectID, a: AudioObjectPropertyAddress, v: &T, what: &str) -> Result<(), AudioError> {
    check(AudioObjectSetPropertyData(obj, &a, 0, ptr::null(), mem::size_of::<T>() as u32, v as *const T as *const c_void), what)
}

unsafe fn get_vec<T: Copy>(obj: AudioObjectID, a: AudioObjectPropertyAddress, what: &str) -> Result<Vec<T>, AudioError> {
    let mut size = 0u32;
    check(AudioObjectGetPropertyDataSize(obj, &a, 0, ptr::null(), &mut size), what)?;
    let mut v: Vec<T> = vec![mem::zeroed(); size as usize / mem::size_of::<T>()];
    check(AudioObjectGetPropertyData(obj, &a, 0, ptr::null(), &mut size, v.as_mut_ptr() as *mut c_void), what)?;
    v.truncate(size as usize / mem::size_of::<T>());
    Ok(v)
}

/// Classe un format physique ; `None` = format qu'on ne sait pas produire (on l'ignore).
fn classify(f: &AudioStreamBasicDescription) -> Option<SampleKind> {
    if f.mFormatID != kAudioFormatLinearPCM || f.mChannelsPerFrame == 0 {
        return None;
    }
    if f.mFormatFlags & kAudioFormatFlagIsNonInterleaved != 0 {
        return None;
    }
    let bytes = f.mBytesPerFrame / f.mChannelsPerFrame;
    let flags = f.mFormatFlags;
    if flags & kAudioFormatFlagIsFloat != 0 && f.mBitsPerChannel == 32 {
        return Some(SampleKind::F32);
    }
    if flags & kAudioFormatFlagIsSignedInteger == 0 {
        return None;
    }
    match (f.mBitsPerChannel, bytes) {
        (16, 2) => Some(SampleKind::I16),
        (24, 3) => Some(SampleKind::I24Packed),
        (24, 4) if flags & kAudioFormatFlagIsAlignedHigh != 0 => Some(SampleKind::I32 { valid_bits: 24 }),
        (32, 4) => Some(SampleKind::I32 { valid_bits: 32 }),
        _ => None,
    }
}

/// Préférence : entier ≥ 24 bits (aucune perte vs le décodeur) > 16 bits > float.
fn score(k: SampleKind) -> u32 {
    match k {
        SampleKind::I32 { valid_bits } => 100 + valid_bits as u32,
        SampleKind::I24Packed => 124,
        SampleKind::I16 => 50,
        SampleKind::F32 => 10,
    }
}

unsafe fn pick_physical(stream: AudioObjectID, rate: f64) -> Result<Option<(AudioStreamBasicDescription, SampleKind)>, AudioError> {
    let list: Vec<AudioStreamRangedDescription> =
        get_vec(stream, addr(kAudioStreamPropertyAvailablePhysicalFormats, GLOBAL), "available physical formats")?;
    let mut best: Option<(u32, AudioStreamBasicDescription, SampleKind)> = None;
    for r in list {
        if !(r.mSampleRateRange.mMinimum <= rate && rate <= r.mSampleRateRange.mMaximum) {
            continue;
        }
        let Some(kind) = classify(&r.mFormat) else { continue };
        let mut asbd = r.mFormat;
        asbd.mSampleRate = rate;
        let s = score(kind);
        if best.map_or(true, |(b, _, _)| s > b) {
            best = Some((s, asbd, kind));
        }
    }
    Ok(best.map(|(_, a, k)| (a, k)))
}

unsafe fn device_rates(stream: AudioObjectID) -> Vec<u32> {
    let list: Vec<AudioStreamRangedDescription> =
        get_vec(stream, addr(kAudioStreamPropertyAvailablePhysicalFormats, GLOBAL), "rates").unwrap_or_default();
    audio_core::STANDARD_RATES
        .iter()
        .copied()
        .filter(|&r| list.iter().any(|d| d.mSampleRateRange.mMinimum <= r as f64 && r as f64 <= d.mSampleRateRange.mMaximum))
        .collect()
}

fn wait_until(timeout: Duration, mut cond: impl FnMut() -> bool) -> bool {
    let t0 = Instant::now();
    while t0.elapsed() < timeout {
        if cond() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    cond()
}

/// Contexte du callback temps réel : tout le travail est dans `RenderCtx` (code commun à tous les backends).
struct Ctx {
    render: UnsafeCell<RenderCtx>,
}

unsafe extern "C" fn io_proc(
    _dev: AudioObjectID,
    _now: *const AudioTimeStamp,
    _in: *const AudioBufferList,
    _in_t: *const AudioTimeStamp,
    out: *mut AudioBufferList,
    _out_t: *const AudioTimeStamp,
    client: *mut c_void,
) -> OSStatus {
    let ctx = &*(client as *const Ctx);
    let list = &mut *out;
    if list.mNumberBuffers == 0 {
        return 0;
    }
    let buf = &mut *list.mBuffers.as_mut_ptr(); // flux entrelacé : un seul AudioBuffer
    let bytes = std::slice::from_raw_parts_mut(buf.mData as *mut u8, buf.mDataByteSize as usize);
    (*ctx.render.get()).render(bytes);
    0
}

struct Open {
    dev: AudioObjectID,
    stream: AudioObjectID,
    orig_fmt: AudioStreamBasicDescription,
    hog: bool,
    proc_id: AudioDeviceIOProcID,
    ctx: *mut Ctx,
}

pub struct CoreAudioOutput {
    st: Option<Open>,
}

// SAFETY: les pointeurs bruts ne sont utilisés que par `open`/`close` (même propriétaire)
// et par le thread temps réel du HAL, arrêté avant `Box::from_raw`.
unsafe impl Send for CoreAudioOutput {}

impl CoreAudioOutput {
    pub fn new() -> Self {
        Self { st: None }
    }

    unsafe fn open_inner(&mut self, want: StreamFormat, ring: Arc<Ring>, shared: Arc<Shared>) -> Result<Opened, AudioError> {
        // 1. périphérique + stream
        let mut dev: AudioObjectID = 0;
        get(kAudioObjectSystemObject, addr(kAudioHardwarePropertyDefaultOutputDevice, GLOBAL), &mut dev, "default output")?;
        if dev == 0 {
            return Err(AudioError::NoDevice);
        }
        let streams: Vec<AudioObjectID> = get_vec(dev, addr(kAudioDevicePropertyStreams, OUTPUT), "streams")?;
        let stream = *streams.first().ok_or(AudioError::NoDevice)?;
        let mut orig: AudioStreamBasicDescription = mem::zeroed();
        get(stream, addr(kAudioStreamPropertyPhysicalFormat, GLOBAL), &mut orig, "current physical format")?;

        // 2. format physique à la fréquence exacte (avant de prendre le hog mode : rien à nettoyer si refus)
        let rate = want.rate as f64;
        let (asbd, kind) = pick_physical(stream, rate)?.ok_or_else(|| AudioError::RateUnsupported {
            requested: want.rate,
            suggested: integer_multiples(want.rate).into_iter().filter(|r| device_rates(stream).contains(r)).collect(),
        })?;

        // 3. hog mode
        let me = std::process::id() as i32;
        let mut owner: i32 = -1;
        get(dev, addr(kAudioDevicePropertyHogMode, GLOBAL), &mut owner, "hog mode (read)")?;
        if owner != -1 && owner != me {
            return Err(AudioError::DeviceBusy);
        }
        set(dev, addr(kAudioDevicePropertyHogMode, GLOBAL), &me, "hog mode (set)")?;
        get(dev, addr(kAudioDevicePropertyHogMode, GLOBAL), &mut owner, "hog mode (verify)")?;
        if owner != me {
            return Err(AudioError::DeviceBusy);
        }
        // Dès ici, `close()` sait tout défaire.
        self.st = Some(Open { dev, stream, orig_fmt: orig, hog: true, proc_id: None, ctx: ptr::null_mut() });

        // 4. changement de format physique + attente de la prise d'effet
        set(stream, addr(kAudioStreamPropertyPhysicalFormat, GLOBAL), &asbd, "set physical format")?;
        let applied = wait_until(Duration::from_secs(2), || {
            let mut cur: AudioStreamBasicDescription = mem::zeroed();
            get(stream, addr(kAudioStreamPropertyPhysicalFormat, GLOBAL), &mut cur, "verify format").is_ok()
                && cur.mSampleRate == rate
                && cur.mBitsPerChannel == asbd.mBitsPerChannel
                && cur.mFormatFlags == asbd.mFormatFlags
        });
        if !applied {
            return Err(AudioError::FormatUnsupported(format!("le driver n'a pas appliqué {} Hz / {} bits", want.rate, asbd.mBitsPerChannel)));
        }
        // Horloge du périphérique alignée sur la fréquence (certains drivers ne le font pas seuls).
        let mut nominal: f64 = 0.0;
        get(dev, addr(kAudioDevicePropertyNominalSampleRate, GLOBAL), &mut nominal, "nominal rate")?;
        if nominal != rate {
            set(dev, addr(kAudioDevicePropertyNominalSampleRate, GLOBAL), &rate, "set nominal rate")?;
            wait_until(Duration::from_secs(2), || {
                let mut n: f64 = 0.0;
                get(dev, addr(kAudioDevicePropertyNominalSampleRate, GLOBAL), &mut n, "nominal").is_ok() && n == rate
            });
        }

        // 5. IOProc
        let dst_ch = asbd.mChannelsPerFrame as usize;
        let device = DeviceFormat { rate: want.rate, channels: dst_ch as u16, kind };
        let ctx = Box::into_raw(Box::new(Ctx { render: UnsafeCell::new(RenderCtx::new(ring, shared, want, device)) }));
        self.st.as_mut().unwrap().ctx = ctx;
        let mut proc_id: AudioDeviceIOProcID = None;
        check(AudioDeviceCreateIOProcID(dev, Some(io_proc), ctx as *mut c_void, &mut proc_id), "create IOProc")?;
        self.st.as_mut().unwrap().proc_id = proc_id;
        check(AudioDeviceStart(dev, proc_id), "start device")?;

        // 6. latence totale vers le DAC
        let (mut buf, mut lat, mut safety, mut slat) = (0u32, 0u32, 0u32, 0u32);
        let _ = get(dev, addr(kAudioDevicePropertyBufferFrameSize, GLOBAL), &mut buf, "buffer size");
        let _ = get(dev, addr(kAudioDevicePropertyLatency, OUTPUT), &mut lat, "latency");
        let _ = get(dev, addr(kAudioDevicePropertySafetyOffset, OUTPUT), &mut safety, "safety");
        let _ = get(stream, addr(kAudioStreamPropertyLatency, GLOBAL), &mut slat, "stream latency");

        Ok(Opened {
            device,
            latency_frames: buf + lat + safety + slat,
        })
    }
}

impl ExclusiveOutput for CoreAudioOutput {
    fn open(&mut self, want: StreamFormat, ring: Arc<Ring>, shared: Arc<Shared>) -> Result<Opened, AudioError> {
        self.close();
        let r = unsafe { self.open_inner(want, ring, shared) };
        if r.is_err() {
            self.close(); // ne jamais laisser le hog mode pris après un échec
        }
        r
    }

    fn close(&mut self) {
        let Some(o) = self.st.take() else { return };
        unsafe {
            if o.proc_id.is_some() {
                AudioDeviceStop(o.dev, o.proc_id);
                AudioDeviceDestroyIOProcID(o.dev, o.proc_id);
            }
            if !o.ctx.is_null() {
                drop(Box::from_raw(o.ctx)); // après destruction de l'IOProc : plus aucun accès concurrent
            }
            let _ = set(o.stream, addr(kAudioStreamPropertyPhysicalFormat, GLOBAL), &o.orig_fmt, "restore format");
            if o.hog {
                let release: i32 = -1;
                let _ = set(o.dev, addr(kAudioDevicePropertyHogMode, GLOBAL), &release, "release hog");
            }
        }
    }
}

impl Drop for CoreAudioOutput {
    fn drop(&mut self) {
        self.close();
    }
}

// TODO(prod) :
//  - listener kAudioDevicePropertyDeviceIsAlive / kAudioHardwarePropertyDefaultOutputDevice -> shared.device_lost = true
//  - choix explicite d'un périphérique (UID) au lieu du défaut
//  - kAudioDevicePropertyBufferFrameSize : viser ~10-20 ms pour réduire la latence A/V
//  - optimisation : sur changement de fréquence, ne pas relâcher/reprendre le hog mode entre deux open()
