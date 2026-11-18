//! Windows : WASAPI en mode EXCLUSIF, piloté par événement, via la crate `wasapi` (série 0.15).
//!
//! Les objets COM ne sont pas `Send` : tout vit dans un thread dédié créé par `open()`.
//! Stratégie de format : on *essaie d'ouvrir* (Initialize) dans cet ordre de préférence
//! 24-bit-dans-32, 32-bit, 24-bit packé, 16-bit, avec plusieurs périodes de tampon
//! (certains pilotes HDA/USB refusent la période par défaut : AUDCLNT_E_BUFFER_SIZE_NOT_ALIGNED).
//! Un Initialize qui réussit = le DAC accepte exactement ce format, sans conversion Windows.
//!
//! NB : non compilé dans l'environnement de rédaction (Linux). Les signatures `wasapi` varient
//! entre versions mineures ; ajustez si besoin. Valider avec un vrai DAC USB.

use crate::{AudioError, ExclusiveOutput, Opened};
use audio_core::{integer_multiples, render::RenderCtx, ring::Ring, DeviceFormat, SampleKind, Shared, StreamFormat};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::thread::JoinHandle;
use std::time::Duration;
use wasapi::*;

struct Worker {
    stop: Arc<AtomicBool>,
    handle: JoinHandle<()>,
}

pub struct WasapiExclusive {
    worker: Option<Worker>,
}

impl WasapiExclusive {
    pub fn new() -> Self {
        Self { worker: None }
    }
}

const CANDIDATES: [(usize, usize, SampleKind); 4] = [
    (32, 24, SampleKind::I32 { valid_bits: 24 }),
    (32, 32, SampleKind::I32 { valid_bits: 32 }),
    (24, 24, SampleKind::I24Packed),
    (16, 16, SampleKind::I16),
];

struct Ready {
    client: AudioClient,
    render: AudioRenderClient,
    event: Handle,
    kind: SampleKind,
    dst_ch: usize,
    block_align: usize,
    buffer_frames: usize,
}

fn setup(want: StreamFormat) -> Result<Ready, AudioError> {
    let device = get_default_device(&Direction::Render).map_err(|e| AudioError::Os(format!("default device: {e}")))?;
    let channels = want.channels.max(2) as usize; // les DAC exclusifs sont (quasi) toujours >= stéréo
    let mut last = String::new();

    for (store, valid, kind) in CANDIDATES {
        let fmt = WaveFormat::new(store, valid, &SampleType::Int, want.rate as usize, channels, None);
        let (def_period, min_period) = {
            let c = device.get_iaudioclient().map_err(|e| AudioError::Os(e.to_string()))?;
            c.get_periods().map_err(|e| AudioError::Os(e.to_string()))?
        };
        for period in [def_period, def_period * 2, min_period.max(100_000), 200_000] {
            // Un IAudioClient ne s'initialise qu'une fois : nouveau client à chaque tentative.
            let mut client = device.get_iaudioclient().map_err(|e| AudioError::Os(e.to_string()))?;
            match client.initialize_client(&fmt, period, &Direction::Render, &ShareMode::Exclusive, false) {
                Ok(()) => {
                    let event = client.set_get_eventhandle().map_err(|e| AudioError::Os(e.to_string()))?;
                    let render = client.get_audiorenderclient().map_err(|e| AudioError::Os(e.to_string()))?;
                    let buffer_frames = client.get_bufferframecount().map_err(|e| AudioError::Os(e.to_string()))? as usize;
                    return Ok(Ready { client, render, event, kind, dst_ch: channels, block_align: fmt.get_blockalign() as usize, buffer_frames });
                }
                Err(e) => last = e.to_string(),
            }
        }
    }
    // 0x8889000A = AUDCLNT_E_DEVICE_IN_USE ; 0x88890008 = AUDCLNT_E_UNSUPPORTED_FORMAT
    let l = last.to_uppercase();
    if l.contains("8889000A") {
        Err(AudioError::DeviceBusy)
    } else if l.contains("88890008") {
        Err(AudioError::RateUnsupported { requested: want.rate, suggested: integer_multiples(want.rate) })
    } else {
        Err(AudioError::FormatUnsupported(last))
    }
}

fn run(want: StreamFormat, ring: Arc<Ring>, shared: Arc<Shared>, stop: Arc<AtomicBool>, tx: mpsc::Sender<Result<Opened, AudioError>>) {
    let _ = initialize_mta();
    // TODO(prod) : AvSetMmThreadCharacteristicsW("Pro Audio") pour la priorité MMCSS de ce thread.
    let ready = match setup(want) {
        Ok(r) => r,
        Err(e) => {
            let _ = tx.send(Err(e));
            return;
        }
    };
    let Ready { mut client, render, event, kind, dst_ch, block_align, buffer_frames } = ready;
    let device = DeviceFormat { rate: want.rate, channels: dst_ch as u16, kind };
    let mut rc = RenderCtx::new(ring, shared.clone(), want, device);
    let mut bytes = vec![0u8; buffer_frames * block_align];

    // Le tampon exclusif doit être rempli (silence) avant Start.
    bytes.iter_mut().for_each(|b| *b = 0);
    if render.write_to_device(buffer_frames, &bytes, None).is_err() || client.start_stream().is_err() {
        let _ = tx.send(Err(AudioError::Os("start_stream".into())));
        return;
    }
    let _ = tx.send(Ok(Opened {
        device,
        latency_frames: (buffer_frames * 2) as u32,
    }));

    while !stop.load(Ordering::Relaxed) {
        if event.wait_for_event(200).is_err() {
            continue; // timeout : on re-teste `stop`
        }
        let frames = match client.get_available_space_in_frames() {
            Ok(n) => (n as usize).min(buffer_frames),
            Err(_) => {
                shared.device_lost.store(true, Ordering::Relaxed);
                break;
            }
        };
        rc.render(&mut bytes[..frames * block_align]);
        if render.write_to_device(frames, &bytes[..frames * block_align], None).is_err() {
            shared.device_lost.store(true, Ordering::Relaxed);
            break;
        }
    }
    let _ = client.stop_stream();
}

impl ExclusiveOutput for WasapiExclusive {
    fn open(&mut self, want: StreamFormat, ring: Arc<Ring>, shared: Arc<Shared>) -> Result<Opened, AudioError> {
        self.close();
        let (tx, rx) = mpsc::channel();
        let stop = Arc::new(AtomicBool::new(false));
        let stop2 = stop.clone();
        let handle = std::thread::Builder::new()
            .name("aurex-wasapi".into())
            .spawn(move || run(want, ring, shared, stop2, tx))
            .map_err(|e| AudioError::Os(e.to_string()))?;
        match rx.recv_timeout(Duration::from_secs(5)) {
            Ok(Ok(opened)) => {
                self.worker = Some(Worker { stop, handle });
                Ok(opened)
            }
            Ok(Err(e)) => {
                let _ = handle.join();
                Err(e)
            }
            Err(_) => {
                stop.store(true, Ordering::Relaxed);
                Err(AudioError::Os("timeout à l'ouverture WASAPI".into()))
            }
        }
    }

    fn close(&mut self) {
        if let Some(w) = self.worker.take() {
            w.stop.store(true, Ordering::Relaxed);
            let _ = w.handle.join(); // stop_stream() relâche l'exclusivité
        }
    }
}

impl Drop for WasapiExclusive {
    fn drop(&mut self) {
        self.close();
    }
}
