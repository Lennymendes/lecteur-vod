//! Pont DSP : ABI C stable pour brancher du code natif (C/C++/Rust) sur la boucle audio.
//!
//! Deux points d'accroche, tous deux dans le thread temps réel du DAC :
//!   1. `process`    : buffer **f32 entrelacé, format source**, modifiable sur place, juste
//!                     AVANT la quantification vers le format du DAC.
//!   2. `tap_device` : octets **exacts** envoyés au DAC (après quantification), lecture seule.
//!                     Sert aux vumètres, analyseurs, et à la VÉRIFICATION bit-perfect (hash).
//!
//! Règles pour le code branché :
//!   * `process` et `tap_device` tournent dans le callback temps réel : ni allocation, ni verrou,
//!     ni E/S, ni appel bloquant. Durée ≪ période du buffer du DAC.
//!   * Un `process` actif qui modifie le signal met fin au bit-perfect ; `DspBridge::modifies_signal()`
//!     le signale à l'UI. Sans plugin (ou en bypass), le chemin est identique octet pour octet.
//!   * `prepare` est appelé hors temps réel à CHAQUE (ré)ouverture du DAC (changement de fréquence
//!     compris) avant le démarrage du rendu ; `reset` après un seek/flush ; `release` à la désinstallation.

use std::any::Any;
use std::ffi::c_void;
use std::fmt;
use std::sync::atomic::{AtomicBool, AtomicPtr, Ordering};
use std::time::Duration;

use crate::{DeviceFormat, SampleKind};

pub const AUREX_DSP_ABI_VERSION: u32 = 1;

/// Format vu par `process` (flux source, avant mapping de canaux vers le DAC).
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AurexDspFormat {
    pub sample_rate: u32,
    pub channels: u32,
    /// Plus grand `frames` que `process` recevra.
    pub max_frames: u32,
}

/// Format des octets reçus par `tap_device`.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AurexDeviceFormat {
    pub sample_rate: u32,
    pub channels: u32,
    /// 0 = f32, 1 = i16, 2 = i24 packé (3 octets), 3 = entier 32 bits (justifié à gauche).
    pub sample_kind: u32,
    /// Bits significatifs (16, 24, 32).
    pub valid_bits: u32,
    pub bytes_per_sample: u32,
}

impl From<DeviceFormat> for AurexDeviceFormat {
    fn from(d: DeviceFormat) -> Self {
        let sample_kind = match d.kind {
            SampleKind::F32 => 0,
            SampleKind::I16 => 1,
            SampleKind::I24Packed => 2,
            SampleKind::I32 { .. } => 3,
        };
        AurexDeviceFormat {
            sample_rate: d.rate,
            channels: d.channels as u32,
            sample_kind,
            valid_bits: d.kind.valid_bits() as u32,
            bytes_per_sample: d.kind.bytes_per_sample() as u32,
        }
    }
}

pub type PrepareFn = unsafe extern "C" fn(user: *mut c_void, fmt: *const AurexDspFormat) -> i32;
pub type ProcessFn = unsafe extern "C" fn(user: *mut c_void, interleaved: *mut f32, frames: u32, channels: u32);
pub type TapFn = unsafe extern "C" fn(user: *mut c_void, data: *const u8, bytes: u32, fmt: *const AurexDeviceFormat);
pub type ResetFn = unsafe extern "C" fn(user: *mut c_void);
pub type ReleaseFn = unsafe extern "C" fn(user: *mut c_void);

/// Table de fonctions fournie par le plugin. Copiée par l'hôte à l'installation.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct AurexDspApi {
    pub abi_version: u32,
    /// `sizeof(AurexDspApi)` côté plugin (évolutivité de l'ABI).
    pub struct_size: u32,
    pub user: *mut c_void,
    /// Latence ajoutée par le plugin, en frames (compensée dans l'horloge A/V).
    pub latency_frames: u32,
    pub prepare: Option<PrepareFn>,
    pub process: Option<ProcessFn>,
    pub tap_device: Option<TapFn>,
    pub reset: Option<ResetFn>,
    pub release: Option<ReleaseFn>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum DspError {
    AbiMismatch { host: u32, plugin: u32 },
    BadStruct,
    NoCallbacks,
    PrepareFailed(i32),
    Load(String),
}

impl fmt::Display for DspError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DspError::AbiMismatch { host, plugin } => write!(f, "ABI DSP incompatible (hôte {host}, plugin {plugin})"),
            DspError::BadStruct => write!(f, "struct_size invalide"),
            DspError::NoCallbacks => write!(f, "ni process ni tap_device"),
            DspError::PrepareFailed(c) => write!(f, "prepare() a échoué (code {c})"),
            DspError::Load(s) => write!(f, "chargement du plugin : {s}"),
        }
    }
}
impl std::error::Error for DspError {}

struct Slot {
    api: AurexDspApi,
    /// Garde la bibliothèque dynamique chargée tant que le slot vit.
    _keepalive: Option<Box<dyn Any + Send>>,
}

/// Singleton global : le thread temps réel lit un pointeur atomique, sans verrou.
pub struct DspBridge {
    slot: AtomicPtr<Slot>,
    bypass: AtomicBool,
}

static GLOBAL: DspBridge = DspBridge::new();

impl DspBridge {
    const fn new() -> Self {
        DspBridge { slot: AtomicPtr::new(std::ptr::null_mut()), bypass: AtomicBool::new(false) }
    }

    pub fn global() -> &'static DspBridge {
        &GLOBAL
    }

    /// Installe un plugin (remplace l'éventuel précédent). Thread de contrôle uniquement.
    pub fn install(&self, api: AurexDspApi, keepalive: Option<Box<dyn Any + Send>>) -> Result<(), DspError> {
        if api.abi_version != AUREX_DSP_ABI_VERSION {
            return Err(DspError::AbiMismatch { host: AUREX_DSP_ABI_VERSION, plugin: api.abi_version });
        }
        if (api.struct_size as usize) < std::mem::size_of::<AurexDspApi>() {
            return Err(DspError::BadStruct);
        }
        if api.process.is_none() && api.tap_device.is_none() {
            return Err(DspError::NoCallbacks);
        }
        self.uninstall();
        let new = Box::into_raw(Box::new(Slot { api, _keepalive: keepalive }));
        self.slot.store(new, Ordering::Release);
        Ok(())
    }

    /// Retire le plugin. Attend que le thread temps réel ait quitté le slot (≥ 100 ms ≫ durée d'un
    /// callback), appelle `release`, puis libère. Thread de contrôle uniquement.
    pub fn uninstall(&self) {
        let old = self.slot.swap(std::ptr::null_mut(), Ordering::AcqRel);
        if old.is_null() {
            return;
        }
        std::thread::sleep(Duration::from_millis(100));
        // SAFETY: `old` vient de Box::into_raw et n'est plus atteignable par le thread RT.
        let slot = unsafe { Box::from_raw(old) };
        if let Some(rel) = slot.api.release {
            unsafe { rel(slot.api.user) };
        }
        drop(slot); // décharge la bibliothèque (après release)
    }

    pub fn set_bypass(&self, on: bool) {
        self.bypass.store(on, Ordering::Relaxed);
    }
    pub fn is_bypassed(&self) -> bool {
        self.bypass.load(Ordering::Relaxed)
    }
    pub fn is_installed(&self) -> bool {
        !self.slot.load(Ordering::Acquire).is_null()
    }
    /// Vrai si un `process` actif peut modifier le signal (donc bit-perfect compromis).
    pub fn modifies_signal(&self) -> bool {
        let p = self.slot.load(Ordering::Acquire);
        // SAFETY: lecture seule d'un slot publié ; la libération attend 100 ms (voir uninstall).
        !p.is_null() && !self.is_bypassed() && unsafe { (*p).api.process.is_some() }
    }

    pub fn latency_frames(&self) -> u32 {
        let p = self.slot.load(Ordering::Acquire);
        if p.is_null() || self.is_bypassed() {
            0
        } else {
            unsafe { (*p).api.latency_frames }
        }
    }

    /// Hors temps réel, avant chaque (ré)ouverture du DAC.
    pub fn prepare(&self, fmt: &AurexDspFormat) -> Result<(), DspError> {
        let p = self.slot.load(Ordering::Acquire);
        if p.is_null() {
            return Ok(());
        }
        let api = unsafe { &(*p).api };
        if let Some(f) = api.prepare {
            let rc = unsafe { f(api.user, fmt) };
            if rc != 0 {
                return Err(DspError::PrepareFailed(rc));
            }
        }
        Ok(())
    }

    /// TEMPS RÉEL. Buffer f32 entrelacé du flux source, modifiable.
    #[inline]
    pub fn process(&self, buf: &mut [f32], channels: usize) {
        let p = self.slot.load(Ordering::Acquire);
        if p.is_null() || self.bypass.load(Ordering::Relaxed) {
            return;
        }
        let api = unsafe { &(*p).api };
        if let Some(f) = api.process {
            let frames = (buf.len() / channels.max(1)) as u32;
            unsafe { f(api.user, buf.as_mut_ptr(), frames, channels as u32) };
        }
    }

    /// TEMPS RÉEL. Octets exacts envoyés au DAC (lecture seule).
    #[inline]
    pub fn tap_device(&self, bytes: &[u8], fmt: &AurexDeviceFormat) {
        let p = self.slot.load(Ordering::Acquire);
        if p.is_null() || self.bypass.load(Ordering::Relaxed) {
            return;
        }
        let api = unsafe { &(*p).api };
        if let Some(f) = api.tap_device {
            unsafe { f(api.user, bytes.as_ptr(), bytes.len() as u32, fmt) };
        }
    }

    /// TEMPS RÉEL (appelé depuis le flush) : le plugin doit vider ses états internes.
    pub fn reset(&self) {
        let p = self.slot.load(Ordering::Acquire);
        if p.is_null() {
            return;
        }
        let api = unsafe { &(*p).api };
        if let Some(f) = api.reset {
            unsafe { f(api.user) };
        }
    }
}
