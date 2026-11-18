use crate::{AudioError, ExclusiveOutput};
use audio_core::dsp::{AurexDspFormat, DspBridge};
use audio_core::{ring::Ring, DeviceFormat, Shared, StreamFormat, MAX_BLOCK_FRAMES};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Résultat d'une ouverture réussie.
#[derive(Clone, Copy, Debug)]
pub struct Opened {
    pub device: DeviceFormat,
    /// Latence approximative (frames) entre le ring et le DAC, pour la synchro A/V.
    pub latency_frames: u32,
}

/// Instantané pour l'UI (badge « Exclusif 24 bits / 44,1 kHz · bit-perfect »).
#[derive(Clone, Debug)]
pub struct EngineStatus {
    pub source: StreamFormat,
    pub device: DeviceFormat,
    pub latency_frames: u32,
    pub switches: u32,
    pub underruns: u64,
    pub dsp_installed: bool,
    /// Un DSP actif modifie le signal : le flux n'est plus bit-perfect.
    pub dsp_modifies_signal: bool,
    pub dsp_error: Option<String>,
    pub bit_perfect: bool,
}

pub struct AudioEngine {
    out: Box<dyn ExclusiveOutput>,
    ring: Option<Arc<Ring>>,
    shared: Arc<Shared>,
    cur: Option<StreamFormat>,
    opened: Option<Opened>,
    switches: u32,
    dsp_error: Option<String>,
}

impl AudioEngine {
    pub fn new(out: Box<dyn ExclusiveOutput>) -> Self {
        Self { out, ring: None, shared: Arc::new(Shared::default()), cur: None, opened: None, switches: 0, dsp_error: None }
    }

    /// Horloge/pause/flush partagés : clonable vers d'autres threads (synchro vidéo, UI, contrôle).
    pub fn shared(&self) -> Arc<Shared> {
        self.shared.clone()
    }

    /// Prépare le DAC pour `fmt` (reconfigure si la fréquence/les canaux changent) puis place
    /// l'horloge à `start_secs` en vidant tout reste. Appel standard au début d'un titre ou après un seek.
    /// Retourne `true` si le DAC a été reconfiguré.
    pub fn begin_track(&mut self, fmt: StreamFormat, start_secs: f64) -> Result<bool, AudioError> {
        let switched = self.ensure_format(fmt)?;
        self.flush_to(start_secs);
        Ok(switched)
    }

    /// Reconfigure le DAC si `fmt` diffère du format courant. `true` = reconfiguré.
    pub fn ensure_format(&mut self, fmt: StreamFormat) -> Result<bool, AudioError> {
        if self.cur == Some(fmt) && !self.device_lost() {
            return Ok(false);
        }
        self.reconfigure(fmt)?;
        Ok(true)
    }

    /// Pousse du PCM f32 entrelacé au format `fmt` (reconfigure le DAC si besoin).
    /// Retourne les frames acceptées (le ring peut être plein).
    pub fn push(&mut self, fmt: StreamFormat, pcm: &[f32]) -> Result<usize, AudioError> {
        self.ensure_format(fmt)?;
        Ok(self.ring.as_ref().map_or(0, |r| r.push_frames(pcm, fmt.channels as usize)))
    }

    /// Version bloquante : attend que tout soit écrit (ou `stop`).
    pub fn push_all(&mut self, fmt: StreamFormat, pcm: &[f32], stop: &AtomicBool) -> Result<(), AudioError> {
        let ch = fmt.channels as usize;
        let mut off = 0;
        while off < pcm.len() && !stop.load(Ordering::Relaxed) {
            let n = self.push(fmt, &pcm[off..])?;
            off += n * ch;
            if n == 0 {
                std::thread::sleep(Duration::from_millis(2));
            }
        }
        Ok(())
    }

    /// Vide le ring (côté rendu) et place l'horloge à `secs`. Attend l'acquittement du thread
    /// temps réel (≤ 300 ms) pour garantir qu'aucune donnée périmée ne sera jouée.
    pub fn flush_to(&mut self, secs: f64) {
        if self.cur.is_none() {
            self.shared.fold_epoch(0);
            return;
        }
        self.shared.request_flush(secs);
        let t0 = Instant::now();
        while self.shared.flush_pending() && t0.elapsed() < Duration::from_millis(300) {
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    pub fn set_paused(&self, paused: bool) {
        self.shared.paused.store(paused, Ordering::Relaxed);
    }

    /// Attend que tout ce qui est en file soit sorti du DAC (fin de titre).
    pub fn drain(&mut self, timeout: Duration) {
        let (Some(r), Some(o), Some(c)) = (&self.ring, self.opened, self.cur) else { return };
        if self.shared.paused.load(Ordering::Relaxed) {
            return;
        }
        let t0 = Instant::now();
        while !r.is_empty() && t0.elapsed() < timeout && !self.device_lost() {
            std::thread::sleep(Duration::from_millis(2));
        }
        std::thread::sleep(Duration::from_secs_f64((o.latency_frames as f64 / c.rate as f64).min(0.25)));
    }

    fn reconfigure(&mut self, fmt: StreamFormat) -> Result<(), AudioError> {
        // 1. Laisser finir la fin du flux précédent (sauf pause / périphérique perdu).
        if self.opened.is_some() && !self.device_lost() {
            self.drain(Duration::from_millis(600));
        }
        // 2. Fermer (restaure le DAC, libère l'exclusivité) puis changer d'époque d'horloge.
        self.out.close();
        self.cur = None;
        self.opened = None;
        self.shared.device_lost.store(false, Ordering::Relaxed);
        self.shared.fold_epoch(fmt.rate);

        // 3. Informer le DSP AVANT que le rendu démarre (hors temps réel).
        self.dsp_error = None;
        let dsp = DspBridge::global();
        let dsp_fmt = AurexDspFormat { sample_rate: fmt.rate, channels: fmt.channels as u32, max_frames: MAX_BLOCK_FRAMES as u32 };
        if let Err(e) = dsp.prepare(&dsp_fmt) {
            // Un plugin défaillant ne doit jamais couper le son : bypass + erreur remontée à l'UI.
            dsp.set_bypass(true);
            self.dsp_error = Some(e.to_string());
        }

        // 4. Rouvrir à la nouvelle fréquence : ring de 250 ms.
        let ring = Arc::new(Ring::new((fmt.rate as usize * fmt.channels as usize) / 4));
        let opened = self.out.open(fmt, ring.clone(), self.shared.clone())?;
        self.shared.set_latency(opened.latency_frames + dsp.latency_frames());
        self.ring = Some(ring);
        self.cur = Some(fmt);
        self.opened = Some(opened);
        self.switches += 1;
        Ok(())
    }

    pub fn status(&self) -> Option<EngineStatus> {
        let (source, o) = (self.cur?, self.opened?);
        let dsp = DspBridge::global();
        let modifies = dsp.modifies_signal();
        Some(EngineStatus {
            source,
            device: o.device,
            latency_frames: o.latency_frames,
            switches: self.switches,
            underruns: self.shared.underruns.load(Ordering::Relaxed),
            dsp_installed: dsp.is_installed(),
            dsp_modifies_signal: modifies,
            dsp_error: self.dsp_error.clone(),
            // Bit-perfect = fréquence identique, aucun DSP actif, et un conteneur qui ne tronque pas
            // la sortie du décodeur (≥ 24 bits ou float). Le mono dupliqué en stéréo reste exact.
            bit_perfect: !modifies && source.rate == o.device.rate && o.device.kind.valid_bits() >= 24,
        })
    }

    pub fn current(&self) -> Option<(StreamFormat, Opened)> {
        Some((self.cur?, self.opened?))
    }
    pub fn switches(&self) -> u32 {
        self.switches
    }
    pub fn underruns(&self) -> u64 {
        self.shared.underruns.load(Ordering::Relaxed)
    }
    pub fn device_lost(&self) -> bool {
        self.shared.device_lost.load(Ordering::Relaxed)
    }
    /// Temps de lecture présenté (s) : horloge maître.
    pub fn presented_secs(&self) -> f64 {
        self.shared.presented_secs()
    }

    /// Ferme le DAC et libère l'exclusivité (ex. avant de laisser mpv ouvrir sa propre sortie).
    pub fn close(&mut self) {
        self.out.close();
        self.cur = None;
        self.opened = None;
        self.ring = None;
    }
}

impl Drop for AudioEngine {
    fn drop(&mut self) {
        self.out.close();
    }
}
