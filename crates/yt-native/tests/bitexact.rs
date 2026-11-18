//! PREUVE DE BOUT EN BOUT : un WAV 16 bits connu -> ffmpeg (f32) -> ring -> rendu -> conteneur 24-en-32
//! du DAC. Les octets capturés par le pont DSP (tap) juste avant le DAC doivent être IDENTIQUES aux
//! échantillons d'origine (v << 16). Aucun gain, aucun dither, aucun rééchantillonnage.
#![cfg(not(any(target_os = "macos", target_os = "windows")))]
mod common;
use audio_engine::dsp::{AurexDeviceFormat, AurexDspApi, AUREX_DSP_ABI_VERSION};
use audio_engine::{default_output, AudioEngine, DspBridge};
use std::ffi::c_void;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use yt_native::decode::*;

static CAPTURE: Mutex<Vec<u8>> = Mutex::new(Vec::new());

unsafe extern "C" fn tap(_: *mut c_void, data: *const u8, n: u32, f: *const AurexDeviceFormat) {
    assert_eq!(((*f).sample_kind, (*f).valid_bits), (3, 24));
    CAPTURE.lock().unwrap().extend_from_slice(std::slice::from_raw_parts(data, n as usize));
}

#[test]
fn wav_16bit_reaches_the_dac_bit_exact() {
    if !common::have_ffmpeg() {
        return;
    }
    let wav = common::gen("ref.wav", 44_100, 1.0, 1, &["-c:a", "pcm_s16le"]);
    // échantillons de référence lus directement dans le WAV (chunk « data »)
    let raw = std::fs::read(&wav).unwrap();
    let pos = raw.windows(4).position(|w| w == b"data").unwrap();
    let n = u32::from_le_bytes(raw[pos + 4..pos + 8].try_into().unwrap()) as usize;
    let expected: Vec<i32> = raw[pos + 8..pos + 8 + n].chunks_exact(2).map(|b| (i16::from_le_bytes([b[0], b[1]]) as i32) << 16).collect();
    assert_eq!(expected.len(), 44_100);

    DspBridge::global()
        .install(
            AurexDspApi {
                abi_version: AUREX_DSP_ABI_VERSION,
                struct_size: std::mem::size_of::<AurexDspApi>() as u32,
                user: std::ptr::null_mut(),
                latency_frames: 0,
                prepare: None,
                process: None, // tap seul : le chemin reste intact
                tap_device: Some(tap),
                reset: None,
                release: None,
            },
            None,
        )
        .unwrap();

    let done = Arc::new(Mutex::new(false));
    let d2 = done.clone();
    let mut s = AudioSession::start(
        DecoderConfig::default(),
        common::src(&wav),
        0.0,
        AudioEngine::new(default_output().unwrap()),
        Arc::new(move |e| {
            if matches!(e, AudioEvent::Ended | AudioEvent::Error(_)) {
                *d2.lock().unwrap() = true;
            }
        }),
    );
    for _ in 0..500 {
        if *done.lock().unwrap() {
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    s.stop();
    DspBridge::global().uninstall();

    let cap: Vec<i32> = CAPTURE.lock().unwrap().chunks_exact(4).map(|b| i32::from_le_bytes([b[0], b[1], b[2], b[3]])).collect();
    // alignement sur le premier échantillon non nul, puis comparaison exacte de toute la suite
    let e0 = expected.iter().position(|&v| v != 0).unwrap();
    let c0 = cap.iter().position(|&v| v != 0).expect("rien capturé");
    let len = expected.len() - e0;
    assert!(cap.len() >= c0 + len, "capture trop courte : {} < {}", cap.len(), c0 + len);
    assert_eq!(&cap[c0..c0 + len], &expected[e0..], "les octets envoyés au DAC diffèrent de la source");
}
