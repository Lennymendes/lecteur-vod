//! Briques audio pures (aucune dépendance, testables sur n'importe quel OS).

pub mod convert;
pub mod dsp;
pub mod render;
pub mod ring;

use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};

/// Taille maximale d'un bloc traité par le chemin temps réel (frames).
/// Le pont DSP reçoit des blocs ≤ cette valeur (annoncée dans `prepare`).
pub const MAX_BLOCK_FRAMES: usize = 4096;

/// Format d'un flux *source* tel que le décodeur le produit (PCM f32 entrelacé).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StreamFormat {
    pub rate: u32,
    pub channels: u16,
}

/// Représentation binaire acceptée par le périphérique en mode exclusif.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SampleKind {
    F32,
    I16,
    /// 24 bits packés sur 3 octets (little-endian).
    I24Packed,
    /// Conteneur 32 bits, `valid_bits` significatifs, justifiés à gauche
    /// (cas standard « 24 bits dans 32 » de WASAPI et CoreAudio).
    I32 { valid_bits: u8 },
}

impl SampleKind {
    pub fn bytes_per_sample(self) -> usize {
        match self {
            SampleKind::F32 | SampleKind::I32 { .. } => 4,
            SampleKind::I16 => 2,
            SampleKind::I24Packed => 3,
        }
    }
    pub fn valid_bits(self) -> u8 {
        match self {
            SampleKind::F32 => 32,
            SampleKind::I16 => 16,
            SampleKind::I24Packed => 24,
            SampleKind::I32 { valid_bits } => valid_bits,
        }
    }
}

/// Ce que le DAC a réellement accepté.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DeviceFormat {
    pub rate: u32,
    pub channels: u16,
    pub kind: SampleKind,
}

pub const STANDARD_RATES: [u32; 8] = [44_100, 48_000, 88_200, 96_000, 176_400, 192_000, 352_800, 384_000];

/// Multiples entiers de `src` que l'on peut proposer si le DAC refuse `src`
/// (ex. 44 100 -> 88 200, 176 400). Le suréchantillonnage entier se fait
/// dans l'app (filtre de qualité), jamais par le mixeur de l'OS.
pub fn integer_multiples(src: u32) -> Vec<u32> {
    STANDARD_RATES.iter().copied().filter(|&r| r > src && src != 0 && r % src == 0).collect()
}

/// État partagé, PERSISTANT pendant toute la vie de l'engine (il survit aux changements
/// de fréquence). Écrit par le thread de rendu temps réel, lu depuis n'importe quel thread.
///
/// Horloge : `position = base + frames_played / rate`. Le couple (base, frames) est protégé
/// par un seqlock : lecture cohérente même pendant un flush/changement d'époque, ce qui évite
/// les faux écarts de synchro A/V.
#[derive(Default)]
pub struct Shared {
    /// Frames consommées depuis le début de l'époque courante (hors silence d'underrun).
    pub frames_played: AtomicU64,
    pub underruns: AtomicU64,
    /// Périphérique débranché / invalidé : l'app doit rouvrir.
    pub device_lost: AtomicBool,
    /// Pause : le rendu sort du silence exact et ne consomme plus le ring.
    pub paused: AtomicBool,
    flush_req: AtomicBool,
    flush_target_bits: AtomicU64,
    base_bits: AtomicU64,
    rate: AtomicU32,
    latency_frames: AtomicU32,
    seq: AtomicU64,
}

impl Shared {
    pub fn played(&self) -> u64 {
        self.frames_played.load(Ordering::Relaxed)
    }

    /// Demande (thread de contrôle) : vider le ring et repartir de `to_secs`.
    pub fn request_flush(&self, to_secs: f64) {
        self.flush_target_bits.store(to_secs.to_bits(), Ordering::Relaxed);
        self.flush_req.store(true, Ordering::Release);
    }

    pub fn flush_pending(&self) -> bool {
        self.flush_req.load(Ordering::Acquire)
    }

    /// Thread de rendu : cible du flush demandé, s'il y en a un.
    pub(crate) fn pending_flush_target(&self) -> Option<f64> {
        self.flush_req.load(Ordering::Acquire).then(|| f64::from_bits(self.flush_target_bits.load(Ordering::Relaxed)))
    }

    /// Thread de rendu : flush effectué (ring vidé) -> nouvelle époque.
    pub(crate) fn complete_flush(&self, to_secs: f64) {
        self.set_epoch(to_secs, None);
        self.flush_req.store(false, Ordering::Release);
    }

    fn set_epoch(&self, base_secs: f64, new_rate: Option<u32>) {
        self.seq.fetch_add(1, Ordering::AcqRel); // impair = écriture en cours
        self.base_bits.store(base_secs.to_bits(), Ordering::Relaxed);
        self.frames_played.store(0, Ordering::Relaxed);
        if let Some(r) = new_rate {
            self.rate.store(r, Ordering::Relaxed);
        }
        self.seq.fetch_add(1, Ordering::AcqRel);
    }

    /// Thread de contrôle, rendu ARRÊTÉ : cumule le temps joué puis bascule sur `new_rate`.
    pub fn fold_epoch(&self, new_rate: u32) {
        let (base, frames, rate) = self.snapshot();
        let secs = if rate > 0 { base + frames as f64 / rate as f64 } else { base };
        self.set_epoch(secs, Some(new_rate));
    }

    pub fn set_latency(&self, frames: u32) {
        self.latency_frames.store(frames, Ordering::Relaxed);
    }

    pub fn rate(&self) -> u32 {
        self.rate.load(Ordering::Relaxed)
    }

    fn snapshot(&self) -> (f64, u64, u32) {
        loop {
            let s1 = self.seq.load(Ordering::Acquire);
            if s1 & 1 == 1 {
                std::hint::spin_loop();
                continue;
            }
            let v = (
                f64::from_bits(self.base_bits.load(Ordering::Relaxed)),
                self.frames_played.load(Ordering::Relaxed),
                self.rate.load(Ordering::Relaxed),
            );
            if self.seq.load(Ordering::Acquire) == s1 {
                return v;
            }
        }
    }

    /// Position décodée-consommée par le DAC (s), sans compenser la latence matérielle.
    pub fn position_secs(&self) -> f64 {
        let (base, frames, rate) = self.snapshot();
        if rate == 0 {
            base
        } else {
            base + frames as f64 / rate as f64
        }
    }

    /// Position *réellement audible* (s) : horloge maître pour caler la vidéo.
    pub fn presented_secs(&self) -> f64 {
        let (_, _, rate) = self.snapshot();
        let lat = if rate > 0 { self.latency_frames.load(Ordering::Relaxed) as f64 / rate as f64 } else { 0.0 };
        (self.position_secs() - lat).max(0.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn multiples() {
        assert_eq!(integer_multiples(44_100), vec![88_200, 176_400, 352_800]);
        assert_eq!(integer_multiples(48_000), vec![96_000, 192_000, 384_000]);
        assert!(integer_multiples(384_000).is_empty());
        assert!(integer_multiples(0).is_empty());
    }

    #[test]
    fn clock_folds_across_rate_change() {
        let s = Shared::default();
        s.fold_epoch(44_100);
        s.frames_played.store(44_100, Ordering::Relaxed); // 1 s @ 44,1k
        assert!((s.position_secs() - 1.0).abs() < 1e-9);
        s.fold_epoch(48_000); // bascule : 1 s cumulée
        assert!((s.position_secs() - 1.0).abs() < 1e-9);
        s.frames_played.store(24_000, Ordering::Relaxed); // +0,5 s @ 48k
        assert!((s.position_secs() - 1.5).abs() < 1e-9);
        s.set_latency(4_800);
        assert!((s.presented_secs() - 1.4).abs() < 1e-9);
    }

    #[test]
    fn flush_sets_new_base() {
        let s = Shared::default();
        s.fold_epoch(48_000);
        s.frames_played.store(96_000, Ordering::Relaxed);
        s.request_flush(30.0);
        assert_eq!(s.pending_flush_target(), Some(30.0));
        s.complete_flush(30.0);
        assert!(!s.flush_pending());
        assert!((s.position_secs() - 30.0).abs() < 1e-9);
    }
}
