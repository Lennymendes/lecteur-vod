//! Charges utiles envoyées à l'UI (`emit`). Noms d'événements côté React : voir `ui/src/api.ts`.

use audio_engine::{DeviceFormat, StreamFormat};
use serde::Serialize;

pub const EV_AUDIO: &str = "aurex://audio";
pub const EV_TICK: &str = "aurex://tick";
pub const EV_NOTICE: &str = "aurex://notice";
pub const EV_ERROR: &str = "aurex://error";

/// Ce qui sort réellement vers le DAC : alimente le badge « 24 bits / 44,1 kHz · exclusif · bit-perfect ».
#[derive(Serialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct AudioBadge {
    /// "engine" (pont DSP, bit-perfect vérifiable) ou "mpv" (sortie exclusive de mpv, hors pont DSP).
    pub path: &'static str,
    pub source_rate: u32,
    pub source_channels: u16,
    pub device_rate: u32,
    pub device_bits: u8,
    pub device_float: bool,
    pub exclusive: bool,
    pub bit_perfect: bool,
    pub rate_switched: bool,
    pub dsp_installed: bool,
    pub dsp_modifies_signal: bool,
    pub dsp_bypassed: bool,
}

impl AudioBadge {
    pub fn engine(src: StreamFormat, dev: DeviceFormat, bit_perfect: bool, rate_switched: bool) -> Self {
        let b = audio_engine::DspBridge::global();
        AudioBadge {
            path: "engine",
            source_rate: src.rate,
            source_channels: src.channels,
            device_rate: dev.rate,
            device_bits: dev.kind.valid_bits(),
            device_float: matches!(dev.kind, audio_engine::SampleKind::F32),
            exclusive: true,
            bit_perfect,
            rate_switched,
            dsp_installed: b.is_installed(),
            dsp_modifies_signal: b.modifies_signal(),
            dsp_bypassed: b.is_bypassed(),
        }
    }

    /// Twitch / direct : mpv choisit le format ; on ne peut affirmer « bit-perfect » que de façon limitée.
    pub fn mpv(source_rate: u32, rate: u32, fmt: &str) -> Self {
        AudioBadge {
            path: "mpv",
            source_rate,
            device_rate: rate,
            rate_switched: source_rate == rate,
            device_bits: match fmt {
                "s16" | "s16p" => 16,
                "s24" | "s24p" => 24,
                "s32" | "s32p" => 32,
                _ => 0,
            },
            device_float: fmt.starts_with("float"),
            exclusive: true,
            ..Default::default()
        }
    }
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Tick {
    pub position: f64,
    pub duration: Option<f64>,
    pub paused: bool,
    pub underruns: u64,
    pub title: Option<String>,
    pub source: &'static str, // "youtube" | "twitch" | "idle"
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Notice {
    /// "sponsor-skip" | "ad-backup" | "ad-stripped" | "ad-clean" | "video-resync" | "info"
    pub kind: &'static str,
    pub text: String,
    pub from: Option<f64>,
    pub to: Option<f64>,
}
