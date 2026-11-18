//! Asservissement A/V : l'horloge AUDIO (DAC) est maître ; la vidéo (mpv, sans audio, lecture en
//! `video-sync=desync`) est rattrapée par micro-variations de vitesse (±5 %, imperceptible sur une vidéo muette) ou,
//! au-delà du seuil, par un saut.

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ServoAction {
    /// Vitesse de lecture vidéo à appliquer (1.0 = nominal).
    Speed(f64),
    /// Écart trop grand : sauter la vidéo à cette position (s).
    HardSeek(f64),
}

#[derive(Clone, Debug)]
pub struct Servo {
    pub hard_threshold: f64,
    pub deadband: f64,
    pub max_adjust: f64,
    pub gain: f64,
}

impl Default for Servo {
    fn default() -> Self {
        Servo { hard_threshold: 0.15, deadband: 0.012, max_adjust: 0.05, gain: 0.5 }
    }
}

impl Servo {
    /// `video_pos` et `audio_pos` en secondes. Écart > 0 = vidéo EN AVANCE -> ralentir.
    pub fn step(&self, video_pos: f64, audio_pos: f64) -> ServoAction {
        let drift = video_pos - audio_pos;
        if drift.abs() > self.hard_threshold {
            ServoAction::HardSeek(audio_pos)
        } else if drift.abs() < self.deadband {
            ServoAction::Speed(1.0)
        } else {
            ServoAction::Speed(1.0 - (drift * self.gain).clamp(-self.max_adjust, self.max_adjust))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn simulate(start_drift: f64, secs: f64) -> f64 {
        let servo = Servo::default();
        let (mut audio, mut video, mut speed) = (10.0, 10.0 + start_drift, 1.0);
        let dt = 0.1;
        for _ in 0..(secs / dt) as usize {
            audio += dt;
            video += dt * speed;
            match servo.step(video, audio) {
                ServoAction::Speed(s) => speed = s,
                ServoAction::HardSeek(t) => {
                    video = t;
                    speed = 1.0;
                }
            }
        }
        video - audio
    }

    #[test]
    fn converges_from_small_drift_without_seeking() {
        assert!(simulate(0.12, 12.0).abs() < 0.015);
        assert!(simulate(-0.12, 12.0).abs() < 0.015);
    }

    #[test]
    fn hard_seeks_when_far() {
        let s = Servo::default();
        assert_eq!(s.step(15.0, 10.0), ServoAction::HardSeek(10.0));
        assert_eq!(s.step(10.003, 10.0), ServoAction::Speed(1.0));
    }

    #[test]
    fn speed_is_bounded() {
        let s = Servo::default();
        match s.step(10.14, 10.0) {
            ServoAction::Speed(v) => assert!((0.949..=0.951).contains(&v), "{v}"),
            _ => panic!(),
        }
    }
}
