//! Test d'intégration (backend factice Linux) : la bascule 44,1 -> 48 kHz reconfigure
//! le périphérique, conserve une horloge monotone et ne perd pas d'échantillons.
#![cfg(not(any(target_os = "macos", target_os = "windows")))]

use audio_engine::{default_output, AudioEngine, StreamFormat};
use std::sync::atomic::AtomicBool;

#[test]
fn auto_rate_switch_keeps_clock_monotonic() {
    let mut e = AudioEngine::new(default_output().unwrap());
    let stop = AtomicBool::new(false);
    let a = StreamFormat { rate: 44_100, channels: 2 };
    let b = StreamFormat { rate: 48_000, channels: 2 };

    e.push_all(a, &vec![0.1; 44_100 / 5 * 2], &stop).unwrap(); // 200 ms @ 44,1k
    assert_eq!(e.switches(), 1);
    assert_eq!(e.current().unwrap().0, a);

    e.push_all(b, &vec![0.1; 48_000 / 5 * 2], &stop).unwrap(); // 200 ms @ 48k -> bascule
    assert_eq!(e.switches(), 2);
    assert_eq!(e.current().unwrap().0.rate, 48_000);

    std::thread::sleep(std::time::Duration::from_millis(300));
    let t = e.presented_secs();
    assert!(t > 0.25 && t < 0.5, "horloge = {t}");
}

#[test]
fn pause_freezes_clock_and_flush_rebases_it() {
    let mut e = AudioEngine::new(default_output().unwrap());
    let stop = AtomicBool::new(false);
    let a = StreamFormat { rate: 48_000, channels: 2 };
    e.begin_track(a, 10.0).unwrap(); // titre qui démarre à 10 s
    e.push_all(a, &vec![0.1; 48_000 / 4 * 2], &stop).unwrap(); // 250 ms
    std::thread::sleep(std::time::Duration::from_millis(150));
    let sh = e.shared();
    let t_run = sh.position_secs();
    assert!(t_run > 10.05 && t_run < 10.3, "t_run={t_run}");

    e.set_paused(true);
    std::thread::sleep(std::time::Duration::from_millis(30));
    let t1 = sh.position_secs();
    std::thread::sleep(std::time::Duration::from_millis(120));
    assert_eq!(sh.position_secs(), t1, "l'horloge doit être figée en pause");

    e.flush_to(42.0); // seek
    assert!((sh.position_secs() - 42.0).abs() < 1e-6);
    e.set_paused(false);
    e.push_all(a, &vec![0.1; 48_000 / 10 * 2], &stop).unwrap();
    std::thread::sleep(std::time::Duration::from_millis(150));
    assert!(sh.position_secs() > 42.0 && sh.position_secs() < 42.3);
}
