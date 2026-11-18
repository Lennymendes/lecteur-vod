//! Moteur YouTube natif d'Aurex.
//!
//! ```text
//!  URL ──yt-dlp(sidecar)──► formats ──select──► audio-only + video-only
//!                                                 │                 │
//!        ffmpeg(sidecar) ──PCM f32 natif──► audio-engine (DAC exclusif, pont DSP)   libmpv (vidéo seule)
//!                                   ▲ horloge audio = maître │                          ▲
//!                                   └────────── servo A/V ───┴──────────────────────────┘
//!        SponsorBlock ──segments──► sauts exacts (audio + vidéo)
//! ```
//! La fréquence d'échantillonnage est LUE dans le flux réel (sortie de sondage de ffmpeg), jamais
//! supposée ; `audio-engine` reconfigure le DAC à chaque titre dont la fréquence diffère.

pub mod decode;
pub mod format;
pub mod playback;
pub mod servo;
pub mod sponsorblock;

#[cfg(feature = "net")]
pub mod extract;
