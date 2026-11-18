//! Surface vidéo native placée SOUS la webview transparente (Spike 1 — « airspace »).
//!
//! Principe : la fenêtre Tauri est transparente ; on crée une sous-vue/fenêtre enfant native
//! (HWND sur Windows, NSView sur macOS) positionnée sous la webview, et on la donne à libmpv via
//! `wid`. L'UI React réserve un « trou » transparent (`<VideoSlot/>`) dont elle envoie le rectangle.
//! Toutes les opérations natives s'exécutent sur le thread principal.

#[cfg(target_os = "macos")]
mod mac;
#[cfg(not(any(windows, target_os = "macos")))]
mod stub;
#[cfg(windows)]
mod win;

#[cfg(target_os = "macos")]
pub use mac::Surface;
#[cfg(not(any(windows, target_os = "macos")))]
pub use stub::Surface;
#[cfg(windows)]
pub use win::Surface;
