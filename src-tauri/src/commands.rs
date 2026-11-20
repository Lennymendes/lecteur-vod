//! Commandes IPC. Règle d'or : un seul propriétaire de la sortie audio exclusive à la fois
//! (`AppState::stop_all` avant toute nouvelle lecture).

use crate::events::*;
use crate::sidecar;
use crate::state::{AppState, Current, NoVideo, TwitchSession, YtSession};
use audio_engine::{default_output, AudioEngine, DspBridge};
use hls_guard::{Guard, GuardEvent, TwitchHls};
use serde::Serialize;
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager, State};
use yt_native::decode::{AudioEvent, AudioSource, DecoderConfig};
use yt_native::extract::YtDlp;
use yt_native::format::{choose_route, video_id, Policy, Route};
use yt_native::playback::{Playback, PlaybackEvent, PlaybackSink, VideoControl};
use yt_native::sponsorblock::{fetch_segments, DEFAULT_CATEGORIES};

const SPONSORBLOCK_API: &str = "https://sponsor.ajay.app";

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlayInfo {
    pub title: String,
    pub duration: Option<f64>,
    /// "native" (audio-engine, bit-perfect + pont DSP) ou "mpv" (direct : sortie exclusive de mpv).
    pub route: &'static str,
    pub video: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppInfo {
    pub os: &'static str,
    pub libmpv: bool,
    pub video_surface: bool,
    pub yt_dlp: bool,
    pub ffmpeg: bool,
}

#[tauri::command]
pub fn app_info(state: State<'_, AppState>) -> AppInfo {
    AppInfo {
        os: std::env::consts::OS,
        libmpv: cfg!(feature = "libmpv"),
        video_surface: state.video().is_some(),
        yt_dlp: sidecar::path("yt-dlp").exists(),
        ffmpeg: sidecar::path("ffmpeg").exists(),
    }
}

/// Rassemble les journaux utiles dans un seul fichier sur le Bureau et renvoie son chemin.
#[tauri::command]
pub fn export_logs(state: State<'_, AppState>) -> Result<String, String> {
    let dir = crate::log::dir();
    let read = |n: &str| std::fs::read_to_string(dir.join(n)).unwrap_or_default();
    let mpv: Vec<String> = read("mpv.log")
        .lines()
        .filter(|l| !l.contains("[curl]") && !l.contains("Run command") && !l.contains("lua"))
        .map(String::from)
        .collect();
    let keep = mpv.len().saturating_sub(400);
    let info = app_info(state);
    let out = format!(
        "# Aurex {} — {}\nlibmpv={} surface_video={} yt-dlp={} ffmpeg={}\n\n## aurex.log (fin)\n{}\n\n## mpv.log (filtré, fin)\n{}\n",
        env!("CARGO_PKG_VERSION"),
        info.os,
        info.libmpv,
        info.video_surface,
        info.yt_dlp,
        info.ffmpeg,
        read("aurex.log").lines().rev().take(200).collect::<Vec<_>>().into_iter().rev().collect::<Vec<_>>().join("\n"),
        mpv[keep..].join("\n"),
    );
    let home = std::env::var("HOME").or_else(|_| std::env::var("USERPROFILE")).map_err(|e| e.to_string())?;
    let path = std::path::PathBuf::from(home).join("Desktop").join("Aurex-logs.txt");
    std::fs::write(&path, out).map_err(|e| e.to_string())?;
    Ok(path.to_string_lossy().into_owned())
}

// ───────────────────────────── YouTube natif ─────────────────────────────

#[tauri::command]
pub async fn yt_play(
    app: AppHandle,
    url: String,
    start: Option<f64>,
    max_height: Option<u32>,
    external: Option<crate::external::External>,
) -> Result<PlayInfo, String> {
    crate::log::log(format!("yt_play: {url} (max {max_height:?}, externe {:?})", external.as_ref().map(|e| &e.player)));
    let r = tauri::async_runtime::spawn_blocking(move || yt_play_blocking(app, url, start.unwrap_or(0.0), max_height.unwrap_or(1080), external))
        .await
        .map_err(|e| e.to_string())?;
    if let Err(e) = &r {
        crate::log::log(format!("yt_play: ERREUR {e}"));
    }
    r
}

/// yt-dlp a besoin d'un moteur JavaScript (Deno, embarqué) pour les vidéos YouTube depuis fin 2025.
fn yt_dlp() -> YtDlp {
    let deno = sidecar::path("deno");
    let extra_args = if deno.exists() { vec!["--js-runtimes".into(), format!("deno:{}", deno.display())] } else { vec![] };
    crate::log::log(format!("yt-dlp: {} | deno: {}", sidecar::path("yt-dlp").display(), if deno.exists() { deno.display().to_string() } else { "absent".into() }));
    YtDlp { path: sidecar::path("yt-dlp"), extra_args }
}

fn user_agent(h: &yt_native::format::Headers) -> Option<String> {
    h.0.iter().find(|(k, _)| k.eq_ignore_ascii_case("user-agent")).map(|(_, v)| v.clone())
}

fn yt_play_blocking(app: AppHandle, input: String, start: f64, max_height: u32, external: Option<crate::external::External>) -> Result<PlayInfo, String> {
    let st = app.state::<AppState>();
    let id = video_id(&input).ok_or("URL ou identifiant YouTube invalide")?;

    let resolved = yt_dlp().resolve(&id).map_err(|e| format!("extraction : {e}"))?;
    let pol = Policy { max_height, ..Policy::default() };
    let route = choose_route(&resolved, &pol).ok_or("aucun format audio exploitable (flux protégé ou indisponible)")?;
    crate::log::log(format!("yt: « {} » live={} audio={} vidéo={}", resolved.title, resolved.is_live, resolved.audio.len(), resolved.video.len()));

    // Libère le DAC / arrête mpv AVANT d'ouvrir quoi que ce soit.
    st.stop_all(true);

    if let Some(ext) = external {
        let name = match &route {
            Route::MpvFallback { url } => crate::external::launch(&ext, url, None, &resolved.title, &[])?,
            Route::Native { audio, video: Some(v) } => crate::external::launch(&ext, &v.url, Some(&audio.url), &resolved.title, &v.headers.0)?,
            Route::Native { audio, video: None } => crate::external::launch(&ext, &audio.url, None, &resolved.title, &audio.headers.0)?,
        };
        return Ok(PlayInfo { title: format!("{} — dans {name}", resolved.title), duration: resolved.duration, route: "external", video: false });
    }
    let gen = st.generation.load(Ordering::SeqCst);
    let video = st.video();

    match route {
        Route::MpvFallback { url } => {
            let v = video.ok_or("libmpv indisponible : impossible de lire un direct")?;
            v.load_live(url, None)?;
            *st.current.lock().unwrap() = Current::YtLive { title: resolved.title.clone(), paused: false };
            spawn_ticker(app.clone(), gen);
            spawn_mpv_audio_probe(app.clone(), gen);
            Ok(PlayInfo { title: resolved.title, duration: None, route: "mpv", video: true })
        }
        Route::Native { audio, video: vf } => {
            let ctrl: Arc<dyn VideoControl> = match (&video, vf) {
                (Some(v), Some(vf)) => {
                    v.load_video_only(&vf.url, user_agent(&vf.headers).as_deref(), start)?;
                    v.clone()
                }
                _ => Arc::new(NoVideo),
            };
            let has_video = video.is_some() && vf.is_some();

            let segments = fetch_segments(SPONSORBLOCK_API, &id, DEFAULT_CATEGORIES).unwrap_or_default();
            let engine = AudioEngine::new(default_output().map_err(|e| e.to_string())?);
            let cfg = DecoderConfig { ffmpeg: sidecar::path("ffmpeg"), user_agent: user_agent(&audio.headers) };
            let src = AudioSource { url: audio.url.clone(), headers: audio.headers.0.clone() };

            let sink: PlaybackSink = {
                let app = app.clone();
                Arc::new(move |ev| match ev {
                    PlaybackEvent::Audio(AudioEvent::Opened { source, device, bit_perfect, rate_switched, .. }) => {
                        let _ = app.emit(EV_AUDIO, AudioBadge::engine(source, device, bit_perfect, rate_switched));
                    }
                    PlaybackEvent::Audio(AudioEvent::Ended) => {
                        let _ = app.emit(EV_NOTICE, Notice { kind: "ended", text: "Lecture terminée".into(), from: None, to: None });
                    }
                    PlaybackEvent::Audio(AudioEvent::Error(e)) => {
                        let _ = app.emit(EV_ERROR, e);
                    }
                    PlaybackEvent::Skipped { category, from, to } => {
                        let _ = app.emit(EV_NOTICE, Notice { kind: "sponsor-skip", text: category, from: Some(from), to: Some(to) });
                    }
                    PlaybackEvent::VideoResync { drift } => {
                        let _ = app.emit(EV_NOTICE, Notice { kind: "video-resync", text: format!("{drift:+.3}s"), from: None, to: None });
                    }
                })
            };

            let sb = st.sponsorblock.load(Ordering::SeqCst);
            let playback = Playback::start(cfg, src, start, engine, ctrl, segments, sb, sink);
            *st.current.lock().unwrap() = Current::Yt(YtSession { playback, title: resolved.title.clone(), duration: resolved.duration, id });
            spawn_ticker(app.clone(), gen);
            Ok(PlayInfo { title: resolved.title, duration: resolved.duration, route: "native", video: has_video })
        }
    }
}

// ───────────────────────────── Twitch ─────────────────────────────

#[tauri::command]
pub async fn twitch_play(
    app: AppHandle,
    channel: String,
    quality: Option<String>,
    external: Option<crate::external::External>,
) -> Result<PlayInfo, String> {
    let channel = channel.trim().trim_start_matches('#').to_lowercase();
    if channel.is_empty() || !channel.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return Err("nom de chaîne invalide".into());
    }
    let quality = quality.unwrap_or_else(|| "best".into());

    let a = app.clone();
    tauri::async_runtime::spawn_blocking(move || a.state::<AppState>().stop_all(true)).await.map_err(|e| e.to_string())?;

    let st = app.state::<AppState>();
    let gen = st.generation.load(Ordering::SeqCst);
    let video = st.video();
    if video.is_none() && external.is_none() {
        return Err("libmpv indisponible : Twitch nécessite le lecteur vidéo".into());
    }

    let proxy = st.proxy.get_or_try_init(|| async { hls_guard::spawn_proxy().await }).await.map_err(|e| format!("proxy local : {e}"))?;
    let tw = TwitchHls::new(None).map_err(|e| e.to_string())?;
    let guard = Guard::start(tw, &channel, &quality).await.map_err(|e| e.to_string())?;
    let (url, mut events) = proxy.serve(&channel, guard).await;
    proxy.set_enabled(&channel, st.adblock.load(Ordering::SeqCst)).await;

    *st.current.lock().unwrap() = Current::Twitch(TwitchSession { channel: channel.clone(), paused: false });
    let external_name = match (&external, &video) {
        (Some(ext), _) => Some(crate::external::launch(ext, &url, None, &channel, &[])?),
        (None, Some(v)) => {
            v.load_live(&url, None)?;
            None
        }
        (None, None) => unreachable!(),
    };

    // Événements anti-pub -> UI.
    {
        let app = app.clone();
        tauri::async_runtime::spawn(async move {
            while let Ok(ev) = events.recv().await {
                if app.state::<AppState>().generation.load(Ordering::SeqCst) != gen {
                    break;
                }
                let n = match ev {
                    GuardEvent::BackupStream { player_type } => Notice { kind: "ad-backup", text: player_type, from: None, to: None },
                    GuardEvent::Stripped { segments } => Notice { kind: "ad-stripped", text: format!("≈ {} s", segments * 2), from: None, to: None },
                    GuardEvent::Clean => Notice { kind: "ad-clean", text: String::new(), from: None, to: None },
                    GuardEvent::Error(e) => {
                        let _ = app.emit(EV_ERROR, e);
                        continue;
                    }
                };
                let _ = app.emit(EV_NOTICE, n);
            }
        });
    }
    if let Some(name) = external_name {
        // Le proxy anti-pub reste actif tant qu'Aurex tourne : le lecteur externe le lit en local.
        return Ok(PlayInfo { title: format!("{channel} — dans {name} (anti-pub actif)"), duration: None, route: "external", video: false });
    }
    spawn_ticker(app.clone(), gen);
    spawn_mpv_audio_probe(app.clone(), gen);
    Ok(PlayInfo { title: channel, duration: None, route: "mpv", video: true })
}

// ───────────────────────────── Contrôles ─────────────────────────────

#[tauri::command]
pub fn player_pause(state: State<'_, AppState>, paused: bool) {
    let mut cur = state.current.lock().unwrap();
    match &mut *cur {
        Current::Yt(s) => s.playback.pause(paused),
        Current::YtLive { paused: p, .. } => {
            *p = paused;
            if let Some(v) = state.video() {
                v.set_pause(paused);
            }
        }
        Current::Twitch(s) => {
            s.paused = paused;
            if let Some(v) = state.video() {
                v.set_pause(paused);
            }
        }
        Current::Idle => {}
    }
}

#[tauri::command]
pub fn player_seek(state: State<'_, AppState>, secs: f64) {
    if let Current::Yt(s) = &*state.current.lock().unwrap() {
        s.playback.seek(secs);
    }
}

#[tauri::command]
pub async fn player_stop(app: AppHandle) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || app.state::<AppState>().stop_all(true)).await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn set_adblock(state: State<'_, AppState>, on: bool) -> Result<(), String> {
    state.adblock.store(on, Ordering::SeqCst);
    let ch = match &*state.current.lock().unwrap() {
        Current::Twitch(t) => Some(t.channel.clone()),
        _ => None,
    };
    if let (Some(ch), Some(p)) = (ch, state.proxy.get()) {
        p.set_enabled(&ch, on).await;
    }
    Ok(())
}

#[tauri::command]
pub fn set_sponsorblock(state: State<'_, AppState>, on: bool) {
    state.sponsorblock.store(on, Ordering::SeqCst);
    if let Current::Yt(s) = &*state.current.lock().unwrap() {
        s.playback.set_sponsorblock(on);
    }
}

#[tauri::command]
pub fn sponsor_undo(state: State<'_, AppState>) {
    if let Current::Yt(s) = &*state.current.lock().unwrap() {
        s.playback.undo_skip();
    }
}

// ───────────────────────────── Pont DSP ─────────────────────────────

/// Charge une bibliothèque exposant `aurex_dsp_entry` (voir include/aurex_dsp.h).
/// Prise en compte à la prochaine (ré)ouverture du DAC (prochain titre / changement de fréquence).
#[tauri::command]
pub fn dsp_load_plugin(path: String) -> Result<(), String> {
    audio_engine::plugin::load_plugin(&PathBuf::from(path)).map_err(|e| format!("{e:?}"))
}

#[tauri::command]
pub fn dsp_unload_plugin() {
    audio_engine::plugin::unload_plugin();
}

#[tauri::command]
pub fn dsp_set_bypass(on: bool) {
    DspBridge::global().set_bypass(on);
}

// ───────────────────────────── Surface vidéo ─────────────────────────────

/// Rectangle du `VideoSlot` (pixels CSS, relatif à la zone client). Commande synchrone : exécutée sur
/// le thread principal, ce qu'exigent AppKit et la gestion des fenêtres Win32.
#[tauri::command]
pub fn video_set_rect(state: State<'_, AppState>, x: f64, y: f64, w: f64, h: f64, scale: f64) {
    if let Some(s) = &*state.surface.lock().unwrap() {
        s.set_rect(x, y, w, h, scale);
    }
}

// ───────────────────────────── Tâches de fond ─────────────────────────────

fn spawn_ticker(app: AppHandle, gen: u64) {
    tauri::async_runtime::spawn(async move {
        let mut iv = tokio::time::interval(Duration::from_millis(250));
        loop {
            iv.tick().await;
            let st = app.state::<AppState>();
            if st.generation.load(Ordering::SeqCst) != gen {
                break;
            }
            let tick = match &*st.current.lock().unwrap() {
                Current::Yt(s) => Tick {
                    position: s.playback.position(),
                    duration: s.duration,
                    paused: s.playback.shared().paused.load(Ordering::Relaxed),
                    underruns: s.playback.shared().underruns.load(Ordering::Relaxed),
                    title: Some(s.title.clone()),
                    source: "youtube",
                },
                Current::YtLive { title, paused } => Tick {
                    position: st.video().and_then(|v| v.time_pos()).unwrap_or(0.0),
                    duration: None,
                    paused: *paused,
                    underruns: 0,
                    title: Some(title.clone()),
                    source: "youtube",
                },
                Current::Twitch(t) => Tick {
                    position: st.video().and_then(|v| v.time_pos()).unwrap_or(0.0),
                    duration: None,
                    paused: t.paused,
                    underruns: 0,
                    title: Some(t.channel.clone()),
                    source: "twitch",
                },
                Current::Idle => break,
            };
            let _ = app.emit(EV_TICK, tick);
        }
    });
}

/// mpv ouvre sa sortie exclusive de façon asynchrone : on interroge ses paramètres de sortie audio
/// jusqu'à les obtenir, pour alimenter le badge (fréquence/format réellement envoyés au DAC).
fn spawn_mpv_audio_probe(app: AppHandle, gen: u64) {
    tauri::async_runtime::spawn(async move {
        // Formats entiers essayés, dans l'ordre, si le DAC n'est pas à la fréquence de la source.
        const FALLBACK: [&str; 3] = ["s32", "s24", "s16"];
        let mut tried = 0usize;
        for _ in 0..60 {
            tokio::time::sleep(Duration::from_millis(500)).await;
            let st = app.state::<AppState>();
            if st.generation.load(Ordering::SeqCst) != gen {
                return;
            }
            let Some(v) = st.video() else { return };
            let (Some((rate, fmt)), Some(src)) = (v.audio_out(), v.audio_in_rate()) else { continue };
            if src != rate && tried < FALLBACK.len() {
                crate::log::log(format!("mpv: source {src} Hz mais DAC à {rate} Hz ({fmt}) → essai audio-format={}", FALLBACK[tried]));
                v.retry_audio_format(FALLBACK[tried]);
                tried += 1;
                tokio::time::sleep(Duration::from_millis(700)).await;
                continue;
            }
            if src != rate {
                crate::log::log(format!("mpv: aucun format n'offre {src} Hz sur ce DAC ; retour au choix de mpv"));
                v.retry_audio_format("no");
                tokio::time::sleep(Duration::from_millis(700)).await;
                if let Some((r, f)) = v.audio_out() {
                    let _ = app.emit(EV_AUDIO, AudioBadge::mpv(src, r, &f));
                }
                return;
            }
            crate::log::log(format!("mpv: sortie {rate} Hz {fmt} = fréquence source"));
            let _ = app.emit(EV_AUDIO, AudioBadge::mpv(src, rate, &fmt));
            return;
        }
    });
}
