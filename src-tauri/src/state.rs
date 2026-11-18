use crate::surface::Surface;
use crate::video::Video;
use hls_guard::ProxyHandle;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};
use tokio::sync::OnceCell;
use yt_native::playback::{Playback, VideoControl};

/// Pas de vidéo (libmpv absent ou piste audio seule) : l'horloge audio reste maître, rien à asservir.
pub struct NoVideo;
impl VideoControl for NoVideo {
    fn time_pos(&self) -> Option<f64> {
        None
    }
    fn set_speed(&self, _: f64) {}
    fn seek_abs(&self, _: f64) {}
    fn set_pause(&self, _: bool) {}
}

pub struct YtSession {
    pub playback: Playback,
    pub title: String,
    pub duration: Option<f64>,
    pub id: String,
}

pub struct TwitchSession {
    pub channel: String,
    pub paused: bool,
}

#[derive(Default)]
pub enum Current {
    #[default]
    Idle,
    Yt(YtSession),
    /// YouTube direct : mpv lit tout (sortie exclusive de mpv).
    YtLive { title: String, paused: bool },
    Twitch(TwitchSession),
}

pub struct AppState {
    pub surface: Mutex<Option<Surface>>,
    pub video: Mutex<Option<Arc<Video>>>,
    pub current: Mutex<Current>,
    pub proxy: OnceCell<ProxyHandle>,
    /// Interrupteurs (lus par les sessions ; appliqués à la volée).
    pub adblock: AtomicBool,
    pub sponsorblock: AtomicBool,
    /// Incrémenté à chaque nouvelle lecture : invalide les tâches de l'ancienne (événements, sondes).
    pub generation: std::sync::atomic::AtomicU64,
}

impl Default for AppState {
    fn default() -> Self {
        AppState {
            surface: Mutex::new(None),
            video: Mutex::new(None),
            current: Mutex::new(Current::Idle),
            proxy: OnceCell::new(),
            adblock: AtomicBool::new(true),
            sponsorblock: AtomicBool::new(true),
            generation: Default::default(),
        }
    }
}

impl AppState {
    pub fn video(&self) -> Option<Arc<Video>> {
        self.video.lock().unwrap().clone()
    }

    /// Arrête TOUT ce qui peut tenir la sortie audio exclusive : un seul propriétaire à la fois.
    /// L'engine est relâché (drop de `Playback`) AVANT que mpv n'ouvre sa propre sortie, et inversement.
    /// Bloquant (join de threads) : à appeler depuis `spawn_blocking`.
    pub fn stop_all(&self, state_gen: bool) {
        if state_gen {
            self.generation.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }
        let old = std::mem::take(&mut *self.current.lock().unwrap());
        drop(old); // Playback::drop -> arrêt du décodeur + close() du DAC
        if let Some(v) = self.video() {
            v.stop();
        }
    }
}
