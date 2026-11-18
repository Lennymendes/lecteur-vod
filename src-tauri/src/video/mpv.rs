//! libmpv embarqué (surface native fournie via `wid`).
//!
//! Deux usages :
//!  * `load_live`       : Twitch / direct YouTube — mpv lit audio+vidéo et ouvre SA sortie exclusive
//!                        (`wasapi` + `audio-exclusive` sous Windows, `coreaudio_exclusive` sous macOS).
//!  * `load_video_only` : YouTube natif — mpv lit la vidéo SEULE, sans audio (aid=no), en `video-sync=desync`,
//!                        et suit l'horloge de l'`audio-engine` via `VideoControl` (servo de vitesse).

use libmpv2::Mpv;
use std::sync::Mutex;
use yt_native::playback::VideoControl;

pub struct Video {
    mpv: Mutex<Mpv>,
}

impl Video {
    pub fn new(wid: i64) -> Result<Self, String> {
        let mpv = Mpv::with_initializer(|init| {
            if wid != 0 {
                init.set_property("wid", wid)?;
            }
            init.set_property("log-file", crate::log::dir().join("mpv.log").to_string_lossy().as_ref())?;
            init.set_property("msg-level", "all=warn,cplayer=v,vo=v,ao=v")?;
            init.set_property("vo", "gpu")?;
            // macOS : le contexte « auto » n'essaie que Vulkan (MoltenVK, non embarqué → VK_ERROR_INCOMPATIBLE_DRIVER).
            // On force OpenGL (contexte choisi automatiquement par mpv).
            // Non bloquant : une option refusée par cette version de mpv ne doit pas empêcher l'init.
            if cfg!(target_os = "macos") {
                if let Err(e) = init.set_property("gpu-api", "opengl") {
                    crate::log::log(format!("mpv: gpu-api=opengl refusé : {e:?}"));
                }
            }
            init.set_property("hwdec", "auto-safe")?;
            init.set_property("osc", "no")?;
            init.set_property("input-default-bindings", "no")?;
            init.set_property("input-vo-keyboard", "no")?;
            init.set_property("keep-open", "yes")?;
            init.set_property("idle", "yes")?;
            // Sortie audio de mpv (utilisée seulement par load_live) : exclusive, sans traitement.
            init.set_property("ao", if cfg!(windows) { "wasapi" } else { "coreaudio_exclusive" })?;
            init.set_property("audio-exclusive", "yes")?;
            init.set_property("replaygain", "no")?;
            init.set_property("volume", 100i64)?;
            Ok(())
        })
        .map_err(|e| format!("initialisation de libmpv : {e}"))?;
        Ok(Video { mpv: Mutex::new(mpv) })
    }

    fn m(&self) -> std::sync::MutexGuard<'_, Mpv> {
        self.mpv.lock().unwrap()
    }

    pub fn load_live(&self, url: &str, user_agent: Option<&str>) -> Result<(), String> {
        let m = self.m();
        let e = |e: libmpv2::Error| e.to_string();
        m.set_property("aid", "auto").map_err(e)?;
        m.set_property("video-sync", "audio").map_err(e)?;
        m.set_property("start", "none").map_err(e)?;
        m.set_property("speed", 1.0f64).map_err(e)?;
        m.set_property("pause", false).map_err(e)?;
        if let Some(ua) = user_agent {
            m.set_property("user-agent", ua).map_err(e)?;
        }
        m.command("loadfile", &[url, "replace"]).map_err(e)
    }

    /// Charge la vidéo seule, EN PAUSE à `start` ; `Playback` la démarre quand l'audio sort.
    pub fn load_video_only(&self, url: &str, user_agent: Option<&str>, start: f64) -> Result<(), String> {
        let m = self.m();
        let e = |e: libmpv2::Error| e.to_string();
        m.set_property("aid", "no").map_err(e)?;
        m.set_property("video-sync", "desync").map_err(e)?;
        m.set_property("speed", 1.0f64).map_err(e)?;
        m.set_property("start", format!("{start:.3}").as_str()).map_err(e)?;
        m.set_property("pause", true).map_err(e)?;
        if let Some(ua) = user_agent {
            m.set_property("user-agent", ua).map_err(e)?;
        }
        m.command("loadfile", &[url, "replace"]).map_err(e)
    }

    pub fn stop(&self) {
        let _ = self.m().command("stop", &[]);
    }

    /// (fréquence, format) réellement envoyés à la sortie audio de mpv — pour le badge Twitch.
    pub fn audio_out(&self) -> Option<(u32, String)> {
        let m = self.m();
        let rate = m.get_property::<i64>("audio-out-params/samplerate").ok()?;
        let fmt = m.get_property::<String>("audio-out-params/format").unwrap_or_default();
        Some((rate as u32, fmt))
    }
}

impl VideoControl for Video {
    fn time_pos(&self) -> Option<f64> {
        self.m().get_property::<f64>("time-pos").ok()
    }
    fn set_speed(&self, speed: f64) {
        let _ = self.m().set_property("speed", speed);
    }
    fn seek_abs(&self, secs: f64) {
        let _ = self.m().command("seek", &[format!("{secs:.3}").as_str(), "absolute+exact"]);
    }
    fn set_pause(&self, paused: bool) {
        let _ = self.m().set_property("pause", paused);
    }
}
