//! Le pont DSP de bout en bout (backend factice, vrai chemin de rendu) : prepare à chaque
//! fréquence, process modifiable, tap des octets réellement envoyés au DAC, reset au flush,
//! bypass automatique si prepare échoue. Un seul #[test] : le pont est global.
#![cfg(not(any(target_os = "macos", target_os = "windows")))]

use audio_engine::dsp::{AurexDeviceFormat, AurexDspApi, AurexDspFormat, AUREX_DSP_ABI_VERSION};
use audio_engine::{default_output, AudioEngine, DspBridge, StreamFormat};
use std::ffi::c_void;
use std::sync::atomic::{AtomicI32, AtomicU32, Ordering::SeqCst};
use std::sync::atomic::AtomicBool;

static PREPARE_CALLS: AtomicU32 = AtomicU32::new(0);
static LAST_RATE: AtomicU32 = AtomicU32::new(0);
static PROCESS_CALLS: AtomicU32 = AtomicU32::new(0);
static RESETS: AtomicU32 = AtomicU32::new(0);
static TAP_FIRST: AtomicI32 = AtomicI32::new(0);
static TAP_KIND: AtomicU32 = AtomicU32::new(99);
static PREPARE_RC: AtomicI32 = AtomicI32::new(0);

unsafe extern "C" fn prepare(_: *mut c_void, f: *const AurexDspFormat) -> i32 {
    PREPARE_CALLS.fetch_add(1, SeqCst);
    LAST_RATE.store((*f).sample_rate, SeqCst);
    PREPARE_RC.load(SeqCst)
}
unsafe extern "C" fn process_half(_: *mut c_void, buf: *mut f32, frames: u32, ch: u32) {
    PROCESS_CALLS.fetch_add(1, SeqCst);
    for s in std::slice::from_raw_parts_mut(buf, (frames * ch) as usize) {
        *s *= 0.5;
    }
}
unsafe extern "C" fn tap(_: *mut c_void, data: *const u8, bytes: u32, fmt: *const AurexDeviceFormat) {
    TAP_KIND.store((*fmt).sample_kind, SeqCst);
    let d = std::slice::from_raw_parts(data, bytes as usize);
    for c in d.chunks_exact(4) {
        let v = i32::from_le_bytes([c[0], c[1], c[2], c[3]]);
        if v != 0 && TAP_FIRST.load(SeqCst) == 0 {
            TAP_FIRST.store(v, SeqCst);
        }
    }
}
unsafe extern "C" fn reset(_: *mut c_void) {
    RESETS.fetch_add(1, SeqCst);
}

fn api(with_process: bool) -> AurexDspApi {
    AurexDspApi {
        abi_version: AUREX_DSP_ABI_VERSION,
        struct_size: std::mem::size_of::<AurexDspApi>() as u32,
        user: std::ptr::null_mut(),
        latency_frames: 0,
        prepare: Some(prepare),
        process: if with_process { Some(process_half) } else { None },
        tap_device: Some(tap),
        reset: Some(reset),
        release: None,
    }
}

fn play(e: &mut AudioEngine, rate: u32, v: f32) {
    let fmt = StreamFormat { rate, channels: 2 };
    e.push_all(fmt, &vec![v; (rate / 10 * 2) as usize], &AtomicBool::new(false)).unwrap();
    std::thread::sleep(std::time::Duration::from_millis(200));
}

#[test]
fn dsp_bridge_end_to_end() {
    let bridge = DspBridge::global();
    let mut e = AudioEngine::new(default_output().unwrap());

    // 1) Plugin tap seul (process = None) : le chemin doit rester EXACT et bit-perfect.
    bridge.install(api(false), None).unwrap();
    play(&mut e, 44_100, 0.25);
    assert_eq!(LAST_RATE.load(SeqCst), 44_100);
    assert_eq!(TAP_KIND.load(SeqCst), 3);
    // 0,25 en 24 bits = 0x200000, justifié à gauche dans 32 bits = 0x20000000 : octets inchangés
    assert_eq!(TAP_FIRST.load(SeqCst), 0x2000_0000);
    let st = e.status().unwrap();
    assert!(st.dsp_installed && !st.dsp_modifies_signal && st.bit_perfect);

    // 2) Plugin process (gain 0,5) : signal modifié -> bit_perfect = false, tap voit 0,125.
    bridge.install(api(true), None).unwrap();
    TAP_FIRST.store(0, SeqCst);
    let before = PREPARE_CALLS.load(SeqCst);
    play(&mut e, 44_100, 0.25);
    e.flush_to(0.0); // provoque reset()
    assert!(PROCESS_CALLS.load(SeqCst) > 0);
    assert_eq!(TAP_FIRST.load(SeqCst), 0x1000_0000);
    assert!(e.status().unwrap().dsp_modifies_signal && !e.status().unwrap().bit_perfect);
    assert!(RESETS.load(SeqCst) >= 1);

    // 3) Bascule de fréquence : prepare() rappelé avec la nouvelle fréquence AVANT le rendu.
    play(&mut e, 48_000, 0.25);
    assert_eq!(LAST_RATE.load(SeqCst), 48_000);
    assert!(PREPARE_CALLS.load(SeqCst) > before);

    // 4) Bypass : le signal repasse exact.
    bridge.set_bypass(true);
    assert!(!e.status().unwrap().dsp_modifies_signal);
    bridge.set_bypass(false);

    // 5) prepare() en échec : jamais de coupure du son, bypass + erreur remontée.
    PREPARE_RC.store(7, SeqCst);
    play(&mut e, 44_100, 0.25);
    let st = e.status().unwrap();
    assert!(st.dsp_error.is_some() && bridge.is_bypassed() && !st.dsp_modifies_signal);
    PREPARE_RC.store(0, SeqCst);

    // 6) ABI invalide refusée.
    let mut bad = api(true);
    bad.abi_version = 99;
    assert!(bridge.install(bad, None).is_err());

    bridge.uninstall();
    assert!(!bridge.is_installed());
}
