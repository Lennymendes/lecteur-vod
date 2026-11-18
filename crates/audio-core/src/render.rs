//! Cœur du callback temps réel, PARTAGÉ par tous les backends (CoreAudio, WASAPI, factice).
//! Une seule implémentation = un seul comportement à valider, sur toutes les plateformes.
//!
//! Ordre par bloc : [flush éventuel] -> [pause = silence exact] -> pop ring -> silence si underrun
//! -> `DspBridge::process` (f32 source) -> quantification sans dither -> `DspBridge::tap_device`.

use crate::convert::encode;
use crate::dsp::{AurexDeviceFormat, DspBridge};
use crate::ring::Ring;
use crate::{DeviceFormat, Shared, StreamFormat, MAX_BLOCK_FRAMES};
use std::sync::atomic::Ordering;
use std::sync::Arc;

pub struct RenderCtx {
    ring: Arc<Ring>,
    shared: Arc<Shared>,
    dsp: &'static DspBridge,
    dev: DeviceFormat,
    dev_c: AurexDeviceFormat,
    src_ch: usize,
    dst_ch: usize,
    scratch: Vec<f32>,
}

impl RenderCtx {
    /// Hors temps réel : toute l'allocation se fait ici.
    pub fn new(ring: Arc<Ring>, shared: Arc<Shared>, src: StreamFormat, dev: DeviceFormat) -> Self {
        RenderCtx {
            ring,
            shared,
            dsp: DspBridge::global(),
            dev,
            dev_c: dev.into(),
            src_ch: src.channels as usize,
            dst_ch: dev.channels as usize,
            scratch: vec![0.0; MAX_BLOCK_FRAMES * src.channels as usize],
        }
    }

    pub fn device_format(&self) -> DeviceFormat {
        self.dev
    }

    /// TEMPS RÉEL. Remplit `out` (octets au format du DAC). Aucune allocation, aucun verrou.
    pub fn render(&mut self, out: &mut [u8]) {
        let bps = self.dev.kind.bytes_per_sample();
        let bpf = self.dst_ch * bps;
        let total = out.len() / bpf;

        if let Some(target) = self.shared.pending_flush_target() {
            self.ring.discard_all();
            self.dsp.reset();
            self.shared.complete_flush(target);
        }
        if self.shared.paused.load(Ordering::Relaxed) {
            out.fill(0); // 0 = silence exact dans tous nos formats
            return;
        }

        let mut done = 0;
        while done < total {
            let n = (total - done).min(MAX_BLOCK_FRAMES);
            let buf = &mut self.scratch[..n * self.src_ch];
            let got = self.ring.pop_frames(buf, self.src_ch);
            if got < n {
                buf[got * self.src_ch..].fill(0.0); // silence, jamais de bruit
                if self.shared.played() > 0 || got > 0 {
                    self.shared.underruns.fetch_add(1, Ordering::Relaxed);
                }
            }
            self.dsp.process(buf, self.src_ch);
            encode(self.dev.kind, buf, self.src_ch, self.dst_ch, &mut out[done * bpf..(done + n) * bpf]);
            self.shared.frames_played.fetch_add(got as u64, Ordering::Relaxed);
            done += n;
        }
        out[total * bpf..].fill(0);
        self.dsp.tap_device(&out[..total * bpf], &self.dev_c);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::SampleKind;

    fn ctx(rate: u32) -> (RenderCtx, Arc<Ring>, Arc<Shared>) {
        let ring = Arc::new(Ring::new(1 << 14));
        let shared = Arc::new(Shared::default());
        shared.fold_epoch(rate);
        let c = RenderCtx::new(
            ring.clone(),
            shared.clone(),
            StreamFormat { rate, channels: 2 },
            DeviceFormat { rate, channels: 2, kind: SampleKind::I16 },
        );
        (c, ring, shared)
    }

    #[test]
    fn passes_samples_exactly_without_dsp() {
        let (mut c, ring, shared) = ctx(44_100);
        let src: Vec<f32> = (0..8).map(|i| (i as f32 * 1000.0) / 32768.0).collect();
        ring.push_frames(&src, 2);
        let mut out = [0u8; 16];
        c.render(&mut out);
        let got: Vec<i16> = out.chunks(2).map(|b| i16::from_le_bytes([b[0], b[1]])).collect();
        assert_eq!(got, (0..8).map(|i| i * 1000).collect::<Vec<i16>>());
        assert_eq!(shared.played(), 4);
    }

    #[test]
    fn pause_gives_silence_and_freezes_clock() {
        let (mut c, ring, shared) = ctx(48_000);
        ring.push_frames(&[0.5; 8], 2);
        shared.paused.store(true, Ordering::Relaxed);
        let mut out = [0xAAu8; 16];
        c.render(&mut out);
        assert_eq!(out, [0u8; 16]);
        assert_eq!(shared.played(), 0);
        assert_eq!(ring.len(), 8); // rien consommé
    }

    #[test]
    fn flush_discards_and_rebases_clock() {
        let (mut c, ring, shared) = ctx(48_000);
        ring.push_frames(&[0.5; 64], 2);
        shared.request_flush(12.5);
        let mut out = [0u8; 8];
        c.render(&mut out);
        assert!(ring.is_empty());
        assert!(!shared.flush_pending());
        assert!((shared.position_secs() - 12.5).abs() < 1e-9);
        assert_eq!(out, [0u8; 8]); // après flush : silence (underrun), pas de données périmées
    }
}
