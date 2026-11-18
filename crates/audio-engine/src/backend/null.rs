//! Backend factice : fait tourner le VRAI chemin de rendu (`RenderCtx`) à vitesse temps réel,
//! vers un buffer jeté. Sert aux tests/CI et au développement sans DAC (Linux).

use crate::{AudioError, ExclusiveOutput, Opened};
use audio_core::render::RenderCtx;
use audio_core::{ring::Ring, DeviceFormat, SampleKind, Shared, StreamFormat};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Duration;

pub struct NullOutput {
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl NullOutput {
    pub fn new() -> Self {
        Self { stop: Arc::new(AtomicBool::new(false)), handle: None }
    }
}

impl ExclusiveOutput for NullOutput {
    fn open(&mut self, want: StreamFormat, ring: Arc<Ring>, shared: Arc<Shared>) -> Result<Opened, AudioError> {
        self.close();
        self.stop = Arc::new(AtomicBool::new(false));
        let stop = self.stop.clone();
        let device = DeviceFormat { rate: want.rate, channels: want.channels, kind: SampleKind::I32 { valid_bits: 24 } };
        let mut ctx = RenderCtx::new(ring, shared, want, device);
        let rate = want.rate as f64;
        let max_frames = (want.rate / 20) as usize; // 50 ms par passage au plus
        let bpf = want.channels as usize * 4;
        self.handle = Some(std::thread::spawn(move || {
            // Cadencé sur l'horloge murale (comme un vrai DAC) : le sommeil seul dériverait de plusieurs %.
            let t0 = std::time::Instant::now();
            let mut done: u64 = 0;
            let mut buf = vec![0u8; max_frames * bpf];
            while !stop.load(Ordering::Relaxed) {
                let due = (t0.elapsed().as_secs_f64() * rate) as u64;
                if due > done {
                    let n = ((due - done) as usize).min(max_frames);
                    ctx.render(&mut buf[..n * bpf]);
                    done += n as u64;
                }
                std::thread::sleep(Duration::from_millis(2));
            }
        }));
        Ok(Opened { device, latency_frames: want.rate / 100 })
    }

    fn close(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}
