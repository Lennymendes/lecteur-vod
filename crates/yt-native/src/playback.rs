//! Orchestration d'une lecture YouTube native : audio (DAC exclusif) + vidéo (mpv, sans audio),
//! horloge audio maître, SponsorBlock exact, interrupteurs à la volée.

use crate::decode::{AudioEvent, AudioSession, AudioSource, DecoderConfig, EventSink};
use crate::servo::{Servo, ServoAction};
use crate::sponsorblock::{Segment, Skipper};
use audio_engine::{AudioEngine, Shared};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

/// Ce que le moteur attend du lecteur vidéo (implémenté par libmpv dans src-tauri, factice en test).
pub trait VideoControl: Send + Sync {
    fn time_pos(&self) -> Option<f64>;
    fn set_speed(&self, speed: f64);
    fn seek_abs(&self, secs: f64);
    fn set_pause(&self, paused: bool);
    /// Vidéo en cours de seek ou en attente de données : sa position n'est pas fiable.
    fn stalled(&self) -> bool {
        false
    }
}

#[derive(Clone, Debug)]
pub enum PlaybackEvent {
    Audio(AudioEvent),
    /// Segment SponsorBlock sauté (l'UI propose « Annuler »).
    Skipped { category: String, from: f64, to: f64 },
    /// Vidéo recalée de force (écart > seuil).
    VideoResync { drift: f64 },
}

pub type PlaybackSink = Arc<dyn Fn(PlaybackEvent) + Send + Sync>;

pub struct Playback {
    audio: AudioSession,
    video: Arc<dyn VideoControl>,
    shared: Arc<Shared>,
    stop: Arc<AtomicBool>,
    sb_enabled: Arc<AtomicBool>,
    skipper: Arc<Mutex<Skipper>>,
    /// Instant (ms depuis `t0`) avant lequel le servo ne corrige pas (juste après un seek).
    holdoff_ms: Arc<AtomicU64>,
    t0: Instant,
    sync: Option<JoinHandle<()>>,
}

impl Playback {
    /// La vidéo doit être déjà chargée et EN PAUSE à `start` ; elle est démarrée dès que le DAC joue.
    pub fn start(
        cfg: DecoderConfig,
        src: AudioSource,
        start: f64,
        engine: AudioEngine,
        video: Arc<dyn VideoControl>,
        segments: Vec<Segment>,
        sb_enabled: bool,
        sink: PlaybackSink,
    ) -> Playback {
        let audio_sink: EventSink = {
            let sink = sink.clone();
            Arc::new(move |e| sink(PlaybackEvent::Audio(e)))
        };
        let audio = AudioSession::start(cfg, src, start, engine, audio_sink);
        let shared = audio.shared();
        let stop = Arc::new(AtomicBool::new(false));
        let sb = Arc::new(AtomicBool::new(sb_enabled));
        let skipper = Arc::new(Mutex::new(Skipper::new(segments)));
        let holdoff = Arc::new(AtomicU64::new(0));
        let t0 = Instant::now();

        let sync = {
            let (video, shared, stop, sb, skipper, holdoff) = (video.clone(), shared.clone(), stop.clone(), sb.clone(), skipper.clone(), holdoff.clone());
            let audio_seek: Box<dyn Fn(f64) + Send> = Box::new(audio.seeker());
            thread::Builder::new()
                .name("aurex-sync".into())
                .spawn(move || sync_loop(video, shared, stop, sb, skipper, holdoff, t0, audio_seek, sink))
                .expect("thread sync")
        };
        Playback { audio, video, shared, stop, sb_enabled: sb, skipper, holdoff_ms: holdoff, t0, sync: Some(sync) }
    }

    pub fn shared(&self) -> Arc<Shared> {
        self.shared.clone()
    }

    pub fn position(&self) -> f64 {
        self.shared.presented_secs()
    }

    pub fn pause(&self, paused: bool) {
        self.audio.set_paused(paused);
        self.video.set_pause(paused);
    }

    pub fn seek(&self, secs: f64) {
        self.skipper.lock().unwrap().user_seeked(secs);
        self.hold();
        self.audio.seek(secs);
        self.video.seek_abs(secs);
    }

    /// Interrupteur SponsorBlock à la volée.
    pub fn set_sponsorblock(&self, on: bool) {
        self.sb_enabled.store(on, Ordering::Relaxed);
    }

    /// « Annuler » après un saut : retour au début du segment, qui ne sera plus sauté.
    pub fn undo_skip(&self) {
        let back = self.skipper.lock().unwrap().undo_last();
        if let Some(t) = back {
            self.hold();
            self.audio.seek(t);
            self.video.seek_abs(t);
        }
    }

    fn hold(&self) {
        self.holdoff_ms.store(self.t0.elapsed().as_millis() as u64 + 700, Ordering::Relaxed);
    }

    pub fn is_finished(&self) -> bool {
        self.audio.is_finished()
    }

    pub fn stop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        self.audio.stop(); // relâche le DAC
        if let Some(h) = self.sync.take() {
            let _ = h.join();
        }
        self.video.set_speed(1.0);
    }
}

impl Drop for Playback {
    fn drop(&mut self) {
        self.stop();
    }
}

#[allow(clippy::too_many_arguments)]
fn sync_loop(
    video: Arc<dyn VideoControl>,
    shared: Arc<Shared>,
    stop: Arc<AtomicBool>,
    sb: Arc<AtomicBool>,
    skipper: Arc<Mutex<Skipper>>,
    holdoff: Arc<AtomicU64>,
    t0: Instant,
    audio_seek: Box<dyn Fn(f64) + Send>,
    sink: PlaybackSink,
) {
    let servo = Servo::default();
    let mut started = false;
    let mut last_speed = 1.0f64;
    // Un seek exact en 1080p prend ~0,3–0,6 s : on vise devant l'audio de ce délai (moyenne mesurée)
    // et on espace les seeks, sinon la vidéo court sans fin derrière une cible déjà dépassée.
    let mut seek_lead = 0.4f64;
    let mut pending_seek: Option<(Instant, f64)> = None;
    let mut last_hard = Instant::now() - Duration::from_secs(10);
    while !stop.load(Ordering::Relaxed) {
        thread::sleep(Duration::from_millis(100));
        let a = shared.presented_secs();

        // Démarrage : la vidéo attend que l'audio ait réellement commencé à sortir.
        if !started {
            if shared.played() > 0 && !shared.paused.load(Ordering::Relaxed) {
                video.set_pause(false);
                started = true;
            }
            continue;
        }
        if shared.paused.load(Ordering::Relaxed) {
            continue;
        }

        // SponsorBlock sur l'horloge AUDIO.
        let decision = skipper.lock().unwrap().check(shared.position_secs(), sb.load(Ordering::Relaxed));
        if let Some(d) = decision {
            holdoff.store(t0.elapsed().as_millis() as u64 + 700, Ordering::Relaxed);
            audio_seek(d.to);
            video.seek_abs(d.to);
            sink(PlaybackEvent::Skipped { category: d.category, from: d.from, to: d.to });
            continue;
        }

        if (t0.elapsed().as_millis() as u64) < holdoff.load(Ordering::Relaxed) {
            continue;
        }
        if video.stalled() {
            continue;
        }
        if let Some((t, _)) = pending_seek.take() {
            // Seek terminé : mise à jour du délai mesuré, puis 1 s de stabilisation.
            seek_lead = (0.7 * seek_lead + 0.3 * t.elapsed().as_secs_f64()).clamp(0.1, 2.0);
            holdoff.store(t0.elapsed().as_millis() as u64 + 1000, Ordering::Relaxed);
            continue;
        }
        if let Some(v) = video.time_pos() {
            let action = match servo.step(v, a) {
                ServoAction::HardSeek(_) if last_hard.elapsed() < Duration::from_secs(3) => {
                    // Trop tôt pour un nouveau seek : rattrapage par la vitesse, au maximum.
                    ServoAction::Speed(if v > a { 1.0 - servo.max_adjust } else { 1.0 + servo.max_adjust })
                }
                other => other,
            };
            match action {
                ServoAction::Speed(s) => {
                    if (s - last_speed).abs() > 0.002 {
                        video.set_speed(s);
                        last_speed = s;
                    }
                }
                ServoAction::HardSeek(t) => {
                    sink(PlaybackEvent::VideoResync { drift: v - a });
                    video.seek_abs(t + seek_lead);
                    video.set_speed(1.0);
                    last_speed = 1.0;
                    last_hard = Instant::now();
                    pending_seek = Some((Instant::now(), t));
                    holdoff.store(t0.elapsed().as_millis() as u64 + 150, Ordering::Relaxed);
                }
            }
        }
    }
}
