//! Décodage par sidecar ffmpeg -> PCM f32 natif -> `AudioEngine`.
//!
//! Pourquoi un sidecar : ffmpeg décode TOUT (AAC, Opus, WebM, MP4 fragmenté DASH…) avec les décodeurs
//! de référence, sans dépendance de compilation. On lui demande du `f32le` SANS `-ar`/`-ac` :
//! la fréquence et les canaux sortent tels que dans le flux (aucun rééchantillonnage). La fréquence
//! réelle est lue dans la sortie de sondage de ffmpeg, puis `AudioEngine::begin_track` reconfigure
//! le DAC si elle diffère du titre précédent (44,1 <-> 48 kHz…).

use audio_engine::{AudioEngine, DeviceFormat, Shared, StreamFormat};
use std::collections::VecDeque;
use std::io::{BufRead, BufReader, Read};
use std::path::PathBuf;
use std::process::{Child, ChildStdout, Command, Stdio};
use std::sync::atomic::Ordering;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender, TryRecvError};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

#[derive(Clone, Debug)]
pub struct AudioSource {
    pub url: String,
    pub headers: Vec<(String, String)>,
}

#[derive(Clone, Debug)]
pub struct DecoderConfig {
    pub ffmpeg: PathBuf,
    pub user_agent: Option<String>,
}

impl Default for DecoderConfig {
    fn default() -> Self {
        DecoderConfig { ffmpeg: PathBuf::from("ffmpeg"), user_agent: None }
    }
}

/// Extrait (fréquence, canaux) d'une ligne `Stream #0:0…: Audio: aac (LC)…, 44100 Hz, stereo, fltp…`.
pub fn parse_stream_line(line: &str) -> Option<StreamFormat> {
    let t = line.trim();
    if !t.starts_with("Stream #") {
        return None;
    }
    let after = t.split_once("Audio:")?.1;
    let parts: Vec<&str> = after.split(',').map(str::trim).collect();
    let i = parts.iter().position(|p| p.ends_with(" Hz"))?;
    let rate: u32 = parts[i].trim_end_matches(" Hz").trim().parse().ok()?;
    let layout = parts.get(i + 1)?.to_lowercase();
    let first = layout.split(|c: char| c == ' ' || c == '(').next().unwrap_or("");
    let channels: u16 = match first {
        "mono" => 1,
        "stereo" => 2,
        "quad" => 4,
        "2.1" | "3.0" => 3,
        "4.0" => 4,
        "4.1" | "5.0" => 5,
        "5.1" | "6.0" => 6,
        "6.1" | "7.0" => 7,
        "7.1" => 8,
        n => n.parse().ok()?, // « 2 channels »
    };
    Some(StreamFormat { rate, channels })
}

pub struct PcmReader {
    child: Child,
    stdout: ChildStdout,
    fmt: StreamFormat,
    buf: Vec<u8>,
    carry: usize,
}

impl PcmReader {
    /// Lance ffmpeg à `start` secondes et attend le sondage du flux (fréquence réelle).
    pub fn spawn(cfg: &DecoderConfig, src: &AudioSource, start: f64) -> Result<Self, String> {
        let mut cmd = Command::new(&cfg.ffmpeg);
        cmd.args(["-hide_banner", "-nostdin", "-loglevel", "info"]);
        if src.url.starts_with("http") {
            if let Some(ua) = &cfg.user_agent {
                cmd.args(["-user_agent", ua]);
            }
            if !src.headers.is_empty() {
                let h: String = src.headers.iter().map(|(k, v)| format!("{k}: {v}\r\n")).collect();
                cmd.args(["-headers", &h]);
            }
            cmd.args(["-reconnect", "1", "-reconnect_streamed", "1", "-reconnect_delay_max", "5"]);
        }
        if start > 0.0 {
            cmd.args(["-ss", &format!("{start:.3}")]);
        }
        cmd.args(["-i", &src.url, "-vn", "-sn", "-dn", "-map", "0:a:0", "-f", "f32le", "-acodec", "pcm_f32le", "pipe:1"]);
        cmd.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
        }
        let mut child = cmd.spawn().map_err(|e| format!("lancement de ffmpeg ({}): {e}", cfg.ffmpeg.display()))?;
        let stdout = child.stdout.take().ok_or("stdout indisponible")?;
        let stderr = child.stderr.take().ok_or("stderr indisponible")?;

        let tail: Arc<Mutex<VecDeque<String>>> = Arc::new(Mutex::new(VecDeque::new()));
        let (ptx, prx) = mpsc::channel::<StreamFormat>();
        let tail2 = tail.clone();
        thread::spawn(move || {
            let mut sent = false;
            for line in BufReader::new(stderr).lines().map_while(Result::ok) {
                if !sent {
                    if let Some(f) = parse_stream_line(&line) {
                        let _ = ptx.send(f);
                        sent = true;
                    }
                }
                let mut t = tail2.lock().unwrap();
                if t.len() >= 6 {
                    t.pop_front();
                }
                t.push_back(line);
            }
        });

        let fmt = match prx.recv_timeout(Duration::from_secs(20)) {
            Ok(f) => f,
            Err(e) => {
                let _ = child.kill();
                let _ = child.wait();
                thread::sleep(Duration::from_millis(50));
                let why = tail.lock().unwrap().iter().cloned().collect::<Vec<_>>().join(" | ");
                return Err(match e {
                    RecvTimeoutError::Timeout => format!("sondage ffmpeg : délai dépassé ({why})"),
                    RecvTimeoutError::Disconnected => format!("ffmpeg a échoué : {why}"),
                });
            }
        };
        Ok(PcmReader { child, stdout, fmt, buf: vec![0; 4096 * fmt.channels as usize * 4], carry: 0 })
    }

    pub fn format(&self) -> StreamFormat {
        self.fmt
    }

    /// Lit des frames COMPLÈTES (≤ 4096) dans `out`. `Ok(0)` = fin du flux.
    pub fn read_frames(&mut self, out: &mut Vec<f32>) -> std::io::Result<usize> {
        let bpf = self.fmt.channels as usize * 4;
        let mut have = self.carry;
        while have < bpf {
            let n = self.stdout.read(&mut self.buf[have..])?;
            if n == 0 {
                return Ok(0);
            }
            have += n;
        }
        let frames = have / bpf;
        out.clear();
        out.extend(self.buf[..frames * bpf].chunks_exact(4).map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]])));
        self.buf.copy_within(frames * bpf..have, 0);
        self.carry = have - frames * bpf;
        Ok(frames)
    }
}

impl Drop for PcmReader {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

// ─────────────────────────── session audio ───────────────────────────

#[derive(Clone, Debug)]
pub enum AudioEvent {
    /// DAC (re)configuré pour ce titre ou après un seek.
    Opened { source: StreamFormat, device: DeviceFormat, bit_perfect: bool, rate_switched: bool, position: f64 },
    Ended,
    Error(String),
}

pub type EventSink = Arc<dyn Fn(AudioEvent) + Send + Sync>;

enum Cmd {
    Seek(f64),
    Stop,
}

pub struct AudioSession {
    tx: Sender<Cmd>,
    shared: Arc<Shared>,
    handle: Option<JoinHandle<()>>,
}

impl AudioSession {
    /// Démarre la lecture à `start` secondes. `engine` est consommé : sa sortie exclusive est
    /// relâchée à la fin de la session (le DAC redevient disponible pour mpv ou un autre titre).
    pub fn start(cfg: DecoderConfig, src: AudioSource, start: f64, engine: AudioEngine, sink: EventSink) -> AudioSession {
        let (tx, rx) = mpsc::channel();
        let shared = engine.shared();
        let handle = thread::Builder::new().name("aurex-audio".into()).spawn(move || run(cfg, src, start, engine, rx, sink)).expect("thread audio");
        AudioSession { tx, shared, handle: Some(handle) }
    }

    pub fn shared(&self) -> Arc<Shared> {
        self.shared.clone()
    }
    pub fn seek(&self, secs: f64) {
        let _ = self.tx.send(Cmd::Seek(secs.max(0.0)));
    }
    /// Fermeture clonable qui envoie un seek à la session (pour le thread de synchro).
    pub fn seeker(&self) -> impl Fn(f64) + Send + 'static {
        let tx = self.tx.clone();
        move |t| {
            let _ = tx.send(Cmd::Seek(t.max(0.0)));
        }
    }
    pub fn set_paused(&self, p: bool) {
        self.shared.paused.store(p, Ordering::Relaxed);
    }
    pub fn stop(&mut self) {
        let _ = self.tx.send(Cmd::Stop);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
    pub fn is_finished(&self) -> bool {
        self.handle.as_ref().map_or(true, |h| h.is_finished())
    }
}

impl Drop for AudioSession {
    fn drop(&mut self) {
        self.stop();
    }
}

fn run(cfg: DecoderConfig, src: AudioSource, start: f64, mut engine: AudioEngine, rx: Receiver<Cmd>, sink: EventSink) {
    let mut seek_to = Some(start);
    let mut reader: Option<PcmReader> = None;
    let mut samples: Vec<f32> = Vec::with_capacity(4096 * 2);
    let mut first = true;

    'outer: loop {
        if let Some(t) = seek_to.take() {
            drop(reader.take()); // tue l'ancien ffmpeg : plus aucune donnée périmée
            let r = match PcmReader::spawn(&cfg, &src, t) {
                Ok(r) => r,
                Err(e) => {
                    sink(AudioEvent::Error(e));
                    break;
                }
            };
            let fmt = r.format();
            match engine.begin_track(fmt, t) {
                Ok(switched) => {
                    if let Some(st) = engine.status() {
                        sink(AudioEvent::Opened { source: fmt, device: st.device, bit_perfect: st.bit_perfect, rate_switched: switched || first, position: t });
                    }
                    first = false;
                }
                Err(e) => {
                    sink(AudioEvent::Error(e.to_string()));
                    break;
                }
            }
            reader = Some(r);
        }
        match rx.try_recv() {
            Ok(Cmd::Seek(t)) => {
                seek_to = Some(t);
                continue;
            }
            Ok(Cmd::Stop) | Err(TryRecvError::Disconnected) => break,
            Err(TryRecvError::Empty) => {}
        }
        let r = reader.as_mut().expect("lecteur ouvert");
        let fmt = r.format();
        let ch = fmt.channels as usize;
        match r.read_frames(&mut samples) {
            Ok(0) => {
                // Fin : en pause, attendre la reprise pour ne pas couper la queue ; puis vider le DAC.
                while engine.shared().paused.load(Ordering::Relaxed) {
                    match rx.try_recv() {
                        Ok(Cmd::Seek(t)) => {
                            seek_to = Some(t);
                            continue 'outer;
                        }
                        Ok(Cmd::Stop) | Err(TryRecvError::Disconnected) => break 'outer,
                        Err(TryRecvError::Empty) => thread::sleep(Duration::from_millis(20)),
                    }
                }
                engine.drain(Duration::from_secs(3));
                sink(AudioEvent::Ended);
                break;
            }
            Ok(_) => {
                let mut off = 0;
                while off < samples.len() {
                    match engine.push(fmt, &samples[off..]) {
                        Ok(n) => {
                            off += n * ch;
                            if n == 0 {
                                match rx.try_recv() {
                                    Ok(Cmd::Seek(t)) => {
                                        seek_to = Some(t);
                                        continue 'outer;
                                    }
                                    Ok(Cmd::Stop) | Err(TryRecvError::Disconnected) => break 'outer,
                                    Err(TryRecvError::Empty) => thread::sleep(Duration::from_millis(2)),
                                }
                            }
                        }
                        Err(e) => {
                            sink(AudioEvent::Error(e.to_string()));
                            break 'outer;
                        }
                    }
                }
            }
            Err(e) => {
                sink(AudioEvent::Error(format!("lecture du flux : {e}")));
                break;
            }
        }
    }
    engine.close();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_ffmpeg_probe_lines() {
        let f = |l| parse_stream_line(l);
        assert_eq!(f("  Stream #0:0[0x1](und): Audio: aac (LC) (mp4a / 0x6134706D), 44100 Hz, stereo, fltp, 130 kb/s (default)"), Some(StreamFormat { rate: 44_100, channels: 2 }));
        assert_eq!(f("  Stream #0:0(eng): Audio: opus, 48000 Hz, stereo, fltp (default)"), Some(StreamFormat { rate: 48_000, channels: 2 }));
        assert_eq!(f("    Stream #0:1: Audio: aac (LC), 96000 Hz, mono, fltp"), Some(StreamFormat { rate: 96_000, channels: 1 }));
        assert_eq!(f("    Stream #0:1: Audio: eac3, 48000 Hz, 5.1(side), fltp"), Some(StreamFormat { rate: 48_000, channels: 6 }));
        assert_eq!(f("    Stream #0:1: Audio: pcm_s24le, 192000 Hz, 2 channels, s32"), Some(StreamFormat { rate: 192_000, channels: 2 }));
        assert_eq!(f("  Stream #0:0: Video: h264, yuv420p, 1920x1080, 30 fps"), None);
        assert_eq!(f("Input #0, mov,mp4,m4a, from 'x.m4a':"), None);
    }
}
