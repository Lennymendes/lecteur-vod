//! Sortie audio exclusive avec bascule automatique de fréquence d'échantillonnage.
//!
//! Chaîne : décodeur -> `AudioEngine::push(fmt, pcm_f32)` -> ring SPSC -> callback temps réel
//! (`audio_core::render::RenderCtx`) -> [pont DSP : process] -> quantification sans dither ->
//! [pont DSP : tap_device] -> DAC (mode exclusif).
//! Aucun mixeur, aucun rééchantillonneur, aucun gain de l'OS n'est dans le chemin.

mod backend;
mod engine;
pub mod plugin;

pub use audio_core::dsp::{self, DspBridge, DspError};
pub use audio_core::{DeviceFormat, SampleKind, Shared, StreamFormat};
pub use backend::default_output;
pub use engine::{AudioEngine, EngineStatus, Opened};

use audio_core::ring::Ring;
use std::fmt;
use std::sync::Arc;

#[derive(Debug)]
pub enum AudioError {
    NoDevice,
    /// Un autre processus tient déjà le périphérique en exclusif.
    DeviceBusy,
    /// Le DAC refuse cette fréquence ; `suggested` = multiples entiers à essayer
    /// (l'appelant peut sur-échantillonner lui-même puis repousser à ce débit).
    RateUnsupported { requested: u32, suggested: Vec<u32> },
    FormatUnsupported(String),
    Os(String),
}

impl fmt::Display for AudioError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AudioError::NoDevice => write!(f, "aucun périphérique de sortie"),
            AudioError::DeviceBusy => write!(f, "périphérique déjà en mode exclusif ailleurs"),
            AudioError::RateUnsupported { requested, suggested } => {
                write!(f, "fréquence {requested} Hz refusée (essayer {suggested:?})")
            }
            AudioError::FormatUnsupported(s) => write!(f, "format refusé : {s}"),
            AudioError::Os(s) => write!(f, "erreur OS : {s}"),
        }
    }
}
impl std::error::Error for AudioError {}

/// Contrat d'un backend de sortie exclusive.
pub trait ExclusiveOutput: Send {
    /// Ouvre le périphérique par défaut *en exclusif* à `want.rate` exactement, démarre un rendu
    /// qui appelle `RenderCtx::render` (ring + shared fournis). Ne doit JAMAIS retomber
    /// silencieusement en mode partagé.
    fn open(&mut self, want: StreamFormat, ring: Arc<Ring>, shared: Arc<Shared>) -> Result<Opened, AudioError>;
    /// Arrête, restaure l'état d'origine du périphérique, libère l'exclusivité.
    fn close(&mut self);
}
