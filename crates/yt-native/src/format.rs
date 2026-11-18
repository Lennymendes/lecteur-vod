//! Modèle de formats + politique de sélection (pure, sans E/S : entièrement testée).

#[derive(Clone, Debug, PartialEq)]
pub struct Headers(pub Vec<(String, String)>);

#[derive(Clone, Debug, PartialEq)]
pub struct AudioFormat {
    pub id: String,
    pub url: String,
    pub codec: String, // « opus », « mp4a.40.2 »…
    pub abr: f64,      // kbit/s
    pub asr: u32,      // Hz annoncé (information ; la vraie valeur vient du sondage ffmpeg)
    pub channels: u16,
    pub protocol: String,
    pub headers: Headers,
}

#[derive(Clone, Debug, PartialEq)]
pub struct VideoFormat {
    pub id: String,
    pub url: String,
    pub codec: String, // « avc1.640028 », « vp09.00.40.08 », « av01… »
    pub width: u32,
    pub height: u32,
    pub fps: f64,
    pub tbr: f64,
    pub protocol: String,
    pub has_audio: bool,
    pub headers: Headers,
}

#[derive(Clone, Debug, Default)]
pub struct Resolved {
    pub id: String,
    pub title: String,
    pub duration: Option<f64>,
    pub is_live: bool,
    /// Flux HLS/DASH pour les directs (lu par mpv, sortie exclusive de mpv).
    pub live_url: Option<String>,
    pub audio: Vec<AudioFormat>,
    pub video: Vec<VideoFormat>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CodecPref {
    Auto,
    Opus,
    Aac,
}

#[derive(Clone, Debug)]
pub struct Policy {
    pub audio: CodecPref,
    pub max_height: u32,
    pub max_fps: f64,
}

impl Default for Policy {
    fn default() -> Self {
        Policy { audio: CodecPref::Auto, max_height: 1080, max_fps: 60.0 }
    }
}

fn is_direct(protocol: &str) -> bool {
    protocol.starts_with("http") && !protocol.contains("m3u8") && !protocol.contains("dash")
}

fn is_opus(c: &str) -> bool {
    c.starts_with("opus")
}

/// Audio : flux HTTP direct stéréo (jamais de downmix implicite), meilleur débit ;
/// à débit égal, Opus (48 kHz) avant AAC ; une préférence explicite l'emporte sur le débit.
pub fn select_audio<'a>(formats: &'a [AudioFormat], pol: &Policy) -> Option<&'a AudioFormat> {
    let pref_ok = |f: &AudioFormat| match pol.audio {
        CodecPref::Auto => true,
        CodecPref::Opus => is_opus(&f.codec),
        CodecPref::Aac => f.codec.starts_with("mp4a"),
    };
    let pick = |only_pref: bool| {
        formats
            .iter()
            .filter(|f| is_direct(&f.protocol) && f.channels <= 2 && (!only_pref || pref_ok(f)))
            .max_by(|a, b| {
                a.abr.partial_cmp(&b.abr).unwrap_or(std::cmp::Ordering::Equal).then((is_opus(&a.codec)).cmp(&is_opus(&b.codec)))
            })
    };
    pick(true).or_else(|| pick(false))
}

fn codec_rank(c: &str) -> u8 {
    // décodage matériel quasi universel : H.264 > VP9 > AV1
    if c.starts_with("avc1") {
        3
    } else if c.starts_with("vp09") || c.starts_with("vp9") {
        2
    } else if c.starts_with("av01") {
        1
    } else {
        0
    }
}

/// Vidéo seule (l'audio vient de notre moteur) : plus haute définition ≤ `max_height`/`max_fps`,
/// puis meilleur codec matériel, puis débit.
pub fn select_video<'a>(formats: &'a [VideoFormat], pol: &Policy) -> Option<&'a VideoFormat> {
    formats
        .iter()
        .filter(|f| is_direct(&f.protocol) && !f.has_audio && f.height > 0 && f.height <= pol.max_height && f.fps <= pol.max_fps + 0.5)
        .max_by(|a, b| {
            a.height
                .cmp(&b.height)
                .then(codec_rank(&a.codec).cmp(&codec_rank(&b.codec)))
                .then(a.fps.partial_cmp(&b.fps).unwrap_or(std::cmp::Ordering::Equal))
                .then(a.tbr.partial_cmp(&b.tbr).unwrap_or(std::cmp::Ordering::Equal))
        })
}

#[derive(Debug, PartialEq)]
pub enum Route<'a> {
    /// Chemin bit-perfect : audio -> audio-engine, vidéo seule -> mpv.
    Native { audio: &'a AudioFormat, video: Option<&'a VideoFormat> },
    /// Direct (ou rien d'exploitable) : mpv lit le flux, avec SA sortie audio exclusive (hors pont DSP).
    MpvFallback { url: &'a str },
}

pub fn choose_route<'a>(r: &'a Resolved, pol: &Policy) -> Option<Route<'a>> {
    if r.is_live {
        return r.live_url.as_deref().map(|url| Route::MpvFallback { url });
    }
    match select_audio(&r.audio, pol) {
        Some(audio) => Some(Route::Native { audio, video: select_video(&r.video, pol) }),
        None => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn a(id: &str, codec: &str, abr: f64, ch: u16, proto: &str) -> AudioFormat {
        AudioFormat { id: id.into(), url: format!("https://x/{id}"), codec: codec.into(), abr, asr: 48_000, channels: ch, protocol: proto.into(), headers: Headers(vec![]) }
    }
    fn v(id: &str, codec: &str, h: u32, fps: f64, tbr: f64) -> VideoFormat {
        VideoFormat { id: id.into(), url: format!("https://x/{id}"), codec: codec.into(), width: h * 16 / 9, height: h, fps, tbr, protocol: "https".into(), has_audio: false, headers: Headers(vec![]) }
    }

    #[test]
    fn audio_prefers_bitrate_then_opus_and_never_downmixes() {
        let f = vec![
            a("140", "mp4a.40.2", 129.5, 2, "https"),
            a("251", "opus", 135.0, 2, "https"),
            a("250", "opus", 70.0, 2, "https"),
            a("256", "mp4a.40.2", 192.0, 6, "https"), // 5.1 : écarté
            a("hls", "mp4a.40.2", 256.0, 2, "m3u8_native"), // manifeste : écarté
        ];
        assert_eq!(select_audio(&f, &Policy::default()).unwrap().id, "251");
        let aac = Policy { audio: CodecPref::Aac, ..Policy::default() };
        assert_eq!(select_audio(&f, &aac).unwrap().id, "140");
        let tie = vec![a("140", "mp4a.40.2", 130.0, 2, "https"), a("251", "opus", 130.0, 2, "https")];
        assert_eq!(select_audio(&tie, &Policy::default()).unwrap().id, "251");
    }

    #[test]
    fn audio_pref_falls_back_when_absent() {
        let f = vec![a("140", "mp4a.40.2", 129.5, 2, "https")];
        let p = Policy { audio: CodecPref::Opus, ..Policy::default() };
        assert_eq!(select_audio(&f, &p).unwrap().id, "140");
    }

    #[test]
    fn video_caps_resolution_and_prefers_hw_codec() {
        let f = vec![
            v("299", "avc1.64002a", 1080, 60.0, 4000.0),
            v("303", "vp09.00.40.08", 1080, 60.0, 2800.0),
            v("308", "vp09.00.50.08", 1440, 60.0, 9000.0), // > max_height
            v("137", "avc1.640028", 1080, 30.0, 2500.0),
        ];
        assert_eq!(select_video(&f, &Policy::default()).unwrap().id, "299");
        let p30 = Policy { max_fps: 30.0, ..Policy::default() };
        assert_eq!(select_video(&f, &p30).unwrap().id, "137");
        let p4k = Policy { max_height: 2160, ..Policy::default() };
        assert_eq!(select_video(&f, &p4k).unwrap().id, "308");
    }

    #[test]
    fn routes() {
        let mut r = Resolved { audio: vec![a("251", "opus", 135.0, 2, "https")], video: vec![v("299", "avc1", 1080, 60.0, 4000.0)], ..Default::default() };
        assert!(matches!(choose_route(&r, &Policy::default()), Some(Route::Native { video: Some(_), .. })));
        r.is_live = true;
        assert_eq!(choose_route(&r, &Policy::default()), None); // direct sans URL : rien d'exploitable
        r.live_url = Some("https://x/live.m3u8".into());
        assert_eq!(choose_route(&r, &Policy::default()), Some(Route::MpvFallback { url: "https://x/live.m3u8" }));
        let none = Resolved::default();
        assert_eq!(choose_route(&none, &Policy::default()), None);
    }
}

/// Identifiant vidéo à partir d'une URL (watch?v=, youtu.be/, shorts/, embed/, live/) ou d'un ID brut.
pub fn video_id(input: &str) -> Option<String> {
    let s = input.trim();
    let is_id = |t: &str| t.len() == 11 && t.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    if is_id(s) {
        return Some(s.to_string());
    }
    if let Some(q) = s.split_once("v=") {
        let id: String = q.1.chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_').collect();
        if is_id(&id) {
            return Some(id);
        }
    }
    for marker in ["youtu.be/", "/shorts/", "/embed/", "/live/"] {
        if let Some((_, rest)) = s.split_once(marker) {
            let id: String = rest.chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_').collect();
            if is_id(&id) {
                return Some(id);
            }
        }
    }
    None
}

#[cfg(test)]
mod id_tests {
    use super::video_id;
    #[test]
    fn extracts_ids() {
        for u in [
            "dQw4w9WgXcQ",
            "https://www.youtube.com/watch?v=dQw4w9WgXcQ&t=42s",
            "https://youtu.be/dQw4w9WgXcQ?si=abc",
            "https://www.youtube.com/shorts/dQw4w9WgXcQ",
            "https://www.youtube.com/embed/dQw4w9WgXcQ",
        ] {
            assert_eq!(video_id(u).as_deref(), Some("dQw4w9WgXcQ"), "{u}");
        }
        assert_eq!(video_id("https://example.com/"), None);
    }
}
