//! Stub sans libmpv (`--no-default-features`) : l'audio natif YouTube fonctionne, pas la vidéo.
use yt_native::playback::VideoControl;

pub struct Video;

impl Video {
    pub fn new(_wid: i64) -> Result<Self, String> {
        Err("libmpv non compilé (feature `libmpv` désactivée)".into())
    }
    pub fn load_live(&self, _url: &str, _ua: Option<&str>) -> Result<(), String> {
        Err("libmpv non compilé".into())
    }
    pub fn load_video_only(&self, _url: &str, _ua: Option<&str>, _headers: &[(String, String)], _start: f64) -> Result<(), String> {
        Err("libmpv non compilé".into())
    }
    pub fn stop(&self) {}
    pub fn audio_out(&self) -> Option<(u32, String)> {
        None
    }
    pub fn audio_in_rate(&self) -> Option<u32> {
        None
    }
    pub fn retry_audio_format(&self, _fmt: &str) {}
}

impl VideoControl for Video {
    fn time_pos(&self) -> Option<f64> {
        None
    }
    fn set_speed(&self, _s: f64) {}
    fn seek_abs(&self, _t: f64) {}
    fn set_pause(&self, _p: bool) {}
}
