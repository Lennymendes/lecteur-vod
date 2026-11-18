//! Localisation des sidecars (yt-dlp, ffmpeg).
//! Bundle : à côté de l'exécutable (Tauri retire le suffixe de triplet). Développement :
//! `src-tauri/binaries/<nom>-<triplet>`. Dernier recours : le PATH.

use std::path::PathBuf;

pub fn path(name: &str) -> PathBuf {
    let exe = format!("{name}{}", std::env::consts::EXE_SUFFIX);
    if let Ok(me) = std::env::current_exe() {
        if let Some(dir) = me.parent() {
            let p = dir.join(&exe);
            if p.exists() {
                return p;
            }
        }
    }
    let dev = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("binaries").join(format!("{name}-{}{}", env!("AUREX_TARGET"), std::env::consts::EXE_SUFFIX));
    if dev.exists() {
        return dev;
    }
    PathBuf::from(name)
}
