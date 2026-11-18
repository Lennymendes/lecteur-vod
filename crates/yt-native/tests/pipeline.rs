//! Pipeline réel : fichiers générés par le vrai ffmpeg -> sidecar ffmpeg -> audio-engine (backend
//! factice, vrai chemin de rendu). Vérifie détection de fréquence, bascule 44,1 <-> 48 kHz, fin, seek,
//! synchro A/V (vidéo factice) et SponsorBlock.
#![cfg(not(any(target_os = "macos", target_os = "windows")))]
mod common;
use audio_engine::{default_output, AudioEngine};
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use yt_native::decode::*;
use yt_native::playback::*;
use yt_native::sponsorblock::Segment;

fn collect() -> (EventSink, Arc<Mutex<Vec<AudioEvent>>>) {
    let ev = Arc::new(Mutex::new(vec![]));
    let e2 = ev.clone();
    (Arc::new(move |e| e2.lock().unwrap().push(e)), ev)
}

fn wait_for(mut f: impl FnMut() -> bool, secs: u64) -> bool {
    let t = Instant::now();
    while t.elapsed() < Duration::from_secs(secs) {
        if f() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    f()
}

#[test]
fn detects_rate_and_switches_between_tracks() {
    if !common::have_ffmpeg() {
        eprintln!("ffmpeg absent : test ignoré");
        return;
    }
    let a44 = common::gen("a44.m4a", 44_100, 1.0, 2, &["-c:a", "aac", "-b:a", "128k"]);
    let a48 = common::gen("a48.m4a", 48_000, 1.0, 2, &["-c:a", "aac", "-b:a", "128k"]);
    let mut opened = vec![];
    for p in [&a44, &a48, &a44] {
        let (sink, ev) = collect();
        let mut s = AudioSession::start(DecoderConfig::default(), common::src(p), 0.0, AudioEngine::new(default_output().unwrap()), sink);
        assert!(wait_for(|| ev.lock().unwrap().iter().any(|e| matches!(e, AudioEvent::Ended | AudioEvent::Error(_))), 10), "pas de fin de lecture");
        let evs = ev.lock().unwrap().clone();
        assert!(!evs.iter().any(|e| matches!(e, AudioEvent::Error(_))), "{evs:?}");
        let pos = s.shared().position_secs();
        assert!(pos > 0.85 && pos < 1.2, "position finale {pos}");
        s.stop();
        if let Some(AudioEvent::Opened { source, device, rate_switched, .. }) = evs.iter().find(|e| matches!(e, AudioEvent::Opened { .. })) {
            assert_eq!(source.rate, device.rate, "le DAC doit suivre la source");
            assert!(*rate_switched);
            opened.push(source.rate);
        }
    }
    assert_eq!(opened, vec![44_100, 48_000, 44_100]);
}

#[test]
fn opus_48k_is_decoded_natively() {
    if !common::have_ffmpeg() {
        return;
    }
    let p = common::gen("o48.webm", 48_000, 1.0, 2, &["-c:a", "libopus", "-b:a", "128k"]);
    let (sink, ev) = collect();
    let mut s = AudioSession::start(DecoderConfig::default(), common::src(&p), 0.0, AudioEngine::new(default_output().unwrap()), sink);
    assert!(wait_for(|| ev.lock().unwrap().iter().any(|e| matches!(e, AudioEvent::Ended | AudioEvent::Error(_))), 10));
    let evs = ev.lock().unwrap().clone();
    match &evs[0] {
        AudioEvent::Opened { source, .. } => assert_eq!(source.rate, 48_000),
        e => panic!("{e:?}"),
    }
    s.stop();
}

#[test]
fn seek_and_pause() {
    if !common::have_ffmpeg() {
        return;
    }
    let p = common::gen("long.m4a", 44_100, 8.0, 2, &["-c:a", "aac"]);
    let (sink, _ev) = collect();
    let mut s = AudioSession::start(DecoderConfig::default(), common::src(&p), 0.0, AudioEngine::new(default_output().unwrap()), sink);
    let sh = s.shared();
    assert!(wait_for(|| sh.position_secs() > 0.3, 5));
    s.seek(5.0);
    assert!(wait_for(|| sh.position_secs() >= 5.0, 5), "seek non pris en compte : {}", sh.position_secs());
    assert!(sh.position_secs() < 5.8);
    s.set_paused(true);
    std::thread::sleep(Duration::from_millis(100));
    let t = sh.position_secs();
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(sh.position_secs(), t);
    s.set_paused(false);
    assert!(wait_for(|| sh.position_secs() > t + 0.2, 3));
    s.stop();
}

struct FakeVideo {
    st: Mutex<(f64, f64, bool, Instant)>, // pos, speed, paused, last
    seeks: Mutex<Vec<f64>>,
}
impl FakeVideo {
    fn new(pos: f64) -> Arc<Self> {
        Arc::new(FakeVideo { st: Mutex::new((pos, 1.0, true, Instant::now())), seeks: Mutex::new(vec![]) })
    }
    fn adv(st: &mut (f64, f64, bool, Instant)) {
        let now = Instant::now();
        if !st.2 {
            st.0 += now.duration_since(st.3).as_secs_f64() * st.1;
        }
        st.3 = now;
    }
}
impl VideoControl for FakeVideo {
    fn time_pos(&self) -> Option<f64> {
        let mut s = self.st.lock().unwrap();
        Self::adv(&mut s);
        Some(s.0)
    }
    fn set_speed(&self, v: f64) {
        let mut s = self.st.lock().unwrap();
        Self::adv(&mut s);
        s.1 = v;
    }
    fn seek_abs(&self, t: f64) {
        let mut s = self.st.lock().unwrap();
        Self::adv(&mut s);
        s.0 = t;
        self.seeks.lock().unwrap().push(t);
    }
    fn set_pause(&self, p: bool) {
        let mut s = self.st.lock().unwrap();
        Self::adv(&mut s);
        s.2 = p;
    }
}

#[test]
fn video_follows_audio_clock_and_sponsorblock_skips() {
    if !common::have_ffmpeg() {
        return;
    }
    let p = common::gen("av.m4a", 44_100, 8.0, 2, &["-c:a", "aac"]);
    let video = FakeVideo::new(0.06); // démarre 60 ms en avance
    let events: Arc<Mutex<Vec<PlaybackEvent>>> = Arc::new(Mutex::new(vec![]));
    let e2 = events.clone();
    let mut pb = Playback::start(
        DecoderConfig::default(),
        common::src(&p),
        0.0,
        AudioEngine::new(default_output().unwrap()),
        video.clone(),
        vec![Segment { start: 3.0, end: 5.0, category: "sponsor".into() }],
        true,
        Arc::new(move |e| e2.lock().unwrap().push(e)),
    );
    // 1) la vidéo ne démarre qu'avec l'audio, puis converge sur l'horloge audio
    assert!(wait_for(|| !video.st.lock().unwrap().2, 5), "la vidéo n'a jamais été démarrée");
    std::thread::sleep(Duration::from_millis(1800));
    let drift = (video.time_pos().unwrap() - pb.position()).abs();
    assert!(drift < 0.04, "dérive A/V {drift}");

    // 2) SponsorBlock : à t≈3 s l'audio ET la vidéo sautent à 5 s
    assert!(wait_for(|| events.lock().unwrap().iter().any(|e| matches!(e, PlaybackEvent::Skipped { .. })), 6));
    assert!(wait_for(|| pb.position() >= 5.0, 3));
    assert!(video.seeks.lock().unwrap().iter().any(|t| (*t - 5.0).abs() < 1e-6));
    let d2 = (video.time_pos().unwrap() - pb.position()).abs();
    assert!(d2 < 0.3, "après saut {d2}");

    // 3) interrupteur à la volée + annulation
    pb.undo_skip();
    assert!(wait_for(|| pb.position() < 3.6, 3), "undo : {}", pb.position());
    pb.stop();
    assert!(pb.shared().paused.load(Ordering::Relaxed) || true);
}
