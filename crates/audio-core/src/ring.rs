//! Ring buffer SPSC (un producteur, un consommateur), sans verrou ni allocation
//! côté consommateur : sûr à utiliser dans un callback audio temps réel.

use std::cell::UnsafeCell;
use std::sync::atomic::{AtomicUsize, Ordering};

pub struct Ring {
    buf: Box<[UnsafeCell<f32>]>,
    mask: usize,
    head: AtomicUsize, // écrit par le producteur
    tail: AtomicUsize, // écrit par le consommateur
}

// SAFETY: un seul producteur appelle `push_frames`, un seul consommateur appelle
// `pop_frames` ; les indices atomiques (Acquire/Release) ordonnent les accès aux cases.
unsafe impl Sync for Ring {}
unsafe impl Send for Ring {}

impl Ring {
    pub fn new(min_samples: usize) -> Self {
        let cap = min_samples.max(2).next_power_of_two();
        Ring {
            buf: (0..cap).map(|_| UnsafeCell::new(0.0)).collect(),
            mask: cap - 1,
            head: AtomicUsize::new(0),
            tail: AtomicUsize::new(0),
        }
    }

    pub fn capacity(&self) -> usize {
        self.mask + 1
    }

    /// Échantillons (pas frames) disponibles en lecture.
    pub fn len(&self) -> usize {
        self.head.load(Ordering::Acquire).wrapping_sub(self.tail.load(Ordering::Acquire))
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Producteur uniquement. N'écrit que des frames entières. Retourne les frames écrites.
    pub fn push_frames(&self, src: &[f32], ch: usize) -> usize {
        let head = self.head.load(Ordering::Relaxed);
        let tail = self.tail.load(Ordering::Acquire);
        let free = self.capacity() - head.wrapping_sub(tail);
        let frames = (src.len() / ch).min(free / ch);
        let n = frames * ch;
        for (i, s) in src[..n].iter().enumerate() {
            // SAFETY: case hors de la zone lisible par le consommateur (voir plus haut).
            unsafe { *self.buf[(head.wrapping_add(i)) & self.mask].get() = *s };
        }
        self.head.store(head.wrapping_add(n), Ordering::Release);
        frames
    }

    /// Consommateur uniquement : jette tout ce qui est en file (flush / seek).
    pub fn discard_all(&self) {
        self.tail.store(self.head.load(Ordering::Acquire), Ordering::Release);
    }

    /// Consommateur uniquement (callback temps réel). Retourne les frames lues.
    pub fn pop_frames(&self, dst: &mut [f32], ch: usize) -> usize {
        let tail = self.tail.load(Ordering::Relaxed);
        let head = self.head.load(Ordering::Acquire);
        let avail = head.wrapping_sub(tail);
        let frames = (dst.len() / ch).min(avail / ch);
        let n = frames * ch;
        for (i, d) in dst[..n].iter_mut().enumerate() {
            // SAFETY: case déjà publiée par le producteur (Release/Acquire).
            *d = unsafe { *self.buf[(tail.wrapping_add(i)) & self.mask].get() };
        }
        self.tail.store(tail.wrapping_add(n), Ordering::Release);
        frames
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn wraps_and_keeps_frames_whole() {
        let r = Ring::new(8);
        let mut out = [0f32; 6];
        for round in 0..50 {
            let data: Vec<f32> = (0..6).map(|i| (round * 6 + i) as f32).collect();
            assert_eq!(r.push_frames(&data, 2), 3);
            assert_eq!(r.pop_frames(&mut out, 2), 3);
            assert_eq!(&out[..], &data[..]);
        }
        // 5 échantillons stéréo = 2 frames entières seulement
        assert_eq!(r.push_frames(&[0.0; 5], 2), 2);
    }

    #[test]
    fn discard_all_empties() {
        let r = Ring::new(16);
        r.push_frames(&[1.0; 8], 2);
        r.discard_all();
        assert!(r.is_empty());
    }

    #[test]
    fn threaded_integrity() {
        let r = Arc::new(Ring::new(1024));
        let p = r.clone();
        let total = 200_000usize;
        let h = std::thread::spawn(move || {
            let mut i = 0usize;
            while i < total {
                let chunk: Vec<f32> = (i..(i + 64).min(total)).map(|v| v as f32).collect();
                let w = p.push_frames(&chunk, 1);
                i += w;
                if w == 0 {
                    std::thread::yield_now();
                }
            }
        });
        let mut next = 0usize;
        let mut buf = [0f32; 100];
        while next < total {
            let n = r.pop_frames(&mut buf, 1);
            for v in &buf[..n] {
                assert_eq!(*v as usize, next);
                next += 1;
            }
        }
        h.join().unwrap();
    }
}
