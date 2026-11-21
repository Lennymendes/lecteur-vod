//! Extraction via le sidecar `yt-dlp` (`-J` : un seul JSON avec tous les formats).
//!
//! On s'appuie sur yt-dlp plutôt que de réimplémenter Innertube : signatures, jetons PO, clients de
//! secours et throttling changent en permanence ; yt-dlp suit ces changements. Mise à jour du
//! sidecar = remplacement d'un binaire (voir `scripts/fetch-sidecars.*`), sans recompiler l'app.

use crate::format::{AudioFormat, Headers, Resolved, VideoFormat};
use serde::Deserialize;
use std::collections::HashMap;
use std::path::PathBuf;
use std::process::{Command, Stdio};

#[derive(Clone, Debug)]
pub struct YtDlp {
    pub path: PathBuf,
    /// Ex. `["--cookies-from-browser", "firefox"]` (vidéos restreintes, compte Premium…).
    pub extra_args: Vec<String>,
}

impl Default for YtDlp {
    fn default() -> Self {
        YtDlp { path: PathBuf::from("yt-dlp"), extra_args: vec![] }
    }
}

#[derive(Deserialize, Default)]
struct RawFormat {
    format_id: Option<String>,
    url: Option<String>,
    acodec: Option<String>,
    vcodec: Option<String>,
    abr: Option<f64>,
    asr: Option<f64>,
    audio_channels: Option<u32>,
    width: Option<u32>,
    height: Option<u32>,
    fps: Option<f64>,
    tbr: Option<f64>,
    protocol: Option<String>,
    http_headers: Option<HashMap<String, String>>,
    available_at: Option<f64>,
    language_preference: Option<i64>,
    format_note: Option<String>,
}

#[derive(Deserialize, Default)]
struct RawInfo {
    id: Option<String>,
    title: Option<String>,
    duration: Option<f64>,
    is_live: Option<bool>,
    manifest_url: Option<String>,
    url: Option<String>,
    formats: Option<Vec<RawFormat>>,
}

fn has(codec: &Option<String>) -> bool {
    codec.as_deref().map_or(false, |c| c != "none" && !c.is_empty())
}

pub fn parse_info(json: &str) -> Result<Resolved, String> {
    let raw: RawInfo = serde_json::from_str(json).map_err(|e| format!("JSON yt-dlp invalide : {e}"))?;
    let mut r = Resolved {
        id: raw.id.unwrap_or_default(),
        title: raw.title.unwrap_or_default(),
        duration: raw.duration,
        is_live: raw.is_live.unwrap_or(false),
        ..Default::default()
    };
    let mut best_live: Option<(f64, String)> = None;
    for f in raw.formats.unwrap_or_default() {
        let (Some(id), Some(url)) = (f.format_id.clone(), f.url.clone()) else { continue };
        let protocol = f.protocol.clone().unwrap_or_default();
        let headers = Headers(f.http_headers.clone().unwrap_or_default().into_iter().collect());
        let (a, v) = (has(&f.acodec), has(&f.vcodec));
        if r.is_live && protocol.contains("m3u8") {
            let score = f.tbr.unwrap_or(0.0);
            if best_live.as_ref().map_or(true, |(s, _)| score > *s) {
                best_live = Some((score, url.clone()));
            }
        }
        if a && !v {
            let note = f.format_note.clone().unwrap_or_default().to_lowercase();
            let dubbed = url.contains("acont%3Ddubbed")
                || url.contains("acont=dubbed")
                || url.contains("acont%3Ddescriptive")
                || note.contains("dubbed")
                || note.contains("descriptive");
            r.audio.push(AudioFormat {
                lang_pref: f.language_preference.unwrap_or(0) as i32,
                dubbed,
                available_at: f.available_at,
                id,
                url,
                codec: f.acodec.clone().unwrap_or_default(),
                abr: f.abr.or(f.tbr).unwrap_or(0.0),
                asr: f.asr.unwrap_or(0.0) as u32,
                channels: f.audio_channels.unwrap_or(2) as u16,
                protocol,
                headers,
            });
        } else if v {
            r.video.push(VideoFormat {
                id,
                url,
                codec: f.vcodec.clone().unwrap_or_default(),
                width: f.width.unwrap_or(0),
                height: f.height.unwrap_or(0),
                fps: f.fps.unwrap_or(30.0),
                tbr: f.tbr.unwrap_or(0.0),
                protocol,
                has_audio: a,
                headers,
                available_at: f.available_at,
            });
        }
    }
    if r.is_live {
        r.live_url = best_live.map(|(_, u)| u).or(raw.manifest_url).or(raw.url);
    }
    Ok(r)
}

impl YtDlp {
    /// Bloquant (à appeler hors du thread UI) : ~1–4 s.
    pub fn resolve(&self, url_or_id: &str) -> Result<Resolved, String> {
        let target = if url_or_id.starts_with("http") { url_or_id.to_string() } else { format!("https://www.youtube.com/watch?v={url_or_id}") };
        let mut cmd = Command::new(&self.path);
        cmd.args(["-J", "-v", "--no-playlist", "--skip-download"]).args(&self.extra_args).arg(&target);
        cmd.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
        }
        let out = cmd.output().map_err(|e| format!("lancement de yt-dlp ({}) : {e}", self.path.display()))?;
        if !out.status.success() {
            let err = String::from_utf8_lossy(&out.stderr);
            return Err(format!("yt-dlp : {}", err.lines().last().unwrap_or("échec")));
        }
        let mut r = parse_info(&String::from_utf8_lossy(&out.stdout))?;
        r.diag = String::from_utf8_lossy(&out.stderr)
            .lines()
            .filter(|l| {
                let l = l.to_lowercase();
                ["js runtime", "jsc", "challenge", "warning", "error", "player", "client", "po token", "yt-dlp version", "throttl"].iter().any(|k| l.contains(k))
            })
            .map(|l| l.chars().take(240).collect())
            .take(40)
            .collect();
        Ok(r)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::format::{choose_route, Policy, Route};

    const SAMPLE: &str = r#"{
      "id":"abc123","title":"Test","duration":212.0,"is_live":false,
      "formats":[
        {"format_id":"140","url":"https://rr/140","acodec":"mp4a.40.2","vcodec":"none","abr":129.5,"asr":44100,"audio_channels":2,"protocol":"https","http_headers":{"User-Agent":"UA"}},
        {"format_id":"251","url":"https://rr/251","acodec":"opus","vcodec":"none","abr":135.2,"asr":48000,"audio_channels":2,"protocol":"https"},
        {"format_id":"299","url":"https://rr/299","acodec":"none","vcodec":"avc1.64002a","width":1920,"height":1080,"fps":60,"tbr":4200,"protocol":"https"},
        {"format_id":"18","url":"https://rr/18","acodec":"mp4a.40.2","vcodec":"avc1.42001E","width":640,"height":360,"fps":30,"tbr":500,"protocol":"https"},
        {"format_id":"sb0","vcodec":"none","acodec":"none"}
      ]}"#;

    #[test]
    fn parses_ytdlp_json_and_routes_native() {
        let r = parse_info(SAMPLE).unwrap();
        assert_eq!((r.audio.len(), r.video.len()), (2, 2)); // storyboard ignoré, 18 = vidéo+audio
        match choose_route(&r, &Policy::default()).unwrap() {
            Route::Native { audio, video } => {
                assert_eq!(audio.id, "251");
                assert_eq!(video.unwrap().id, "299"); // vidéo seule 1080p60, pas le combiné 360p
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn live_goes_to_mpv() {
        let j = r#"{"id":"x","is_live":true,"formats":[{"format_id":"95","url":"https://m/live.m3u8","acodec":"mp4a","vcodec":"avc1","protocol":"m3u8_native","tbr":2000}]}"#;
        let r = parse_info(j).unwrap();
        assert_eq!(choose_route(&r, &Policy::default()), Some(Route::MpvFallback { url: "https://m/live.m3u8" }));
    }
}
