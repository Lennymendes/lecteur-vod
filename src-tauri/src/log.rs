//! Journal de diagnostic : ~/Library/Logs/Aurex/ (macOS) ou %USERPROFILE%\.aurex\ (Windows).
use std::io::Write;
use std::path::PathBuf;

pub fn dir() -> PathBuf {
    let home = std::env::var("HOME").or_else(|_| std::env::var("USERPROFILE")).unwrap_or_else(|_| ".".into());
    let d = if cfg!(target_os = "macos") { PathBuf::from(home).join("Library/Logs/Aurex") } else { PathBuf::from(home).join(".aurex") };
    let _ = std::fs::create_dir_all(&d);
    d
}

pub fn log(msg: impl AsRef<str>) {
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(dir().join("aurex.log")) {
        let _ = writeln!(f, "{}", msg.as_ref());
    }
}
