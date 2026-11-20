#![allow(dead_code)]
use std::path::{Path, PathBuf};
use std::process::Command;

pub fn have_ffmpeg() -> bool {
    Command::new("ffmpeg").arg("-version").output().map(|o| o.status.success()).unwrap_or(false)
}

pub fn tmp(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("aurex-test-{}", std::process::id()));
    std::fs::create_dir_all(&d).unwrap();
    d.join(name)
}

/// Génère une sinusoïde de `secs` secondes à `rate` Hz avec les options de codec données.
pub fn gen(name: &str, rate: u32, secs: f64, ch: u32, codec: &[&str]) -> PathBuf {
    let out = tmp(name);
    let st = Command::new("ffmpeg")
        .args(["-y", "-hide_banner", "-loglevel", "error", "-f", "lavfi", "-i"])
        .arg(format!("sine=frequency=1000:sample_rate={rate}:duration={secs}"))
        .args(["-ac", &ch.to_string()])
        .args(codec)
        .arg(&out)
        .status()
        .unwrap();
    assert!(st.success(), "ffmpeg n'a pas pu générer {name}");
    out
}

pub fn src(p: &Path) -> yt_native::decode::AudioSource {
    yt_native::decode::AudioSource::direct(p.to_string_lossy().into_owned(), vec![])
}
