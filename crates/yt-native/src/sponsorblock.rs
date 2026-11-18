//! SponsorBlock natif : récupération des segments (API à préfixe de hash, vie privée) et logique
//! de saut exacte appliquée à l'horloge AUDIO. La logique est pure et testée.

use std::collections::HashSet;

#[derive(Clone, Debug, PartialEq)]
pub struct Segment {
    pub start: f64,
    pub end: f64,
    pub category: String,
}

pub const DEFAULT_CATEGORIES: &[&str] = &["sponsor", "selfpromo", "interaction", "intro", "outro", "preview", "music_offtopic"];

/// Fusionne les segments qui se chevauchent / se touchent (évite les sauts en chaîne).
pub fn merge(mut segs: Vec<Segment>) -> Vec<Segment> {
    segs.retain(|s| s.end > s.start && s.start >= 0.0);
    segs.sort_by(|a, b| a.start.partial_cmp(&b.start).unwrap());
    let mut out: Vec<Segment> = Vec::new();
    for s in segs {
        match out.last_mut() {
            Some(l) if s.start <= l.end + 0.05 => {
                l.end = l.end.max(s.end);
            }
            _ => out.push(s),
        }
    }
    out
}

pub struct Skipper {
    segs: Vec<Segment>,
    /// Segments déjà sautés (ne pas re-sauter en boucle) ; `undo` les neutralise.
    done: HashSet<usize>,
    ignored: HashSet<usize>,
    last: Option<usize>,
}

#[derive(Debug, PartialEq)]
pub struct SkipDecision {
    pub to: f64,
    pub category: String,
    pub from: f64,
}

impl Skipper {
    pub fn new(segs: Vec<Segment>) -> Self {
        Skipper { segs: merge(segs), done: HashSet::new(), ignored: HashSet::new(), last: None }
    }

    pub fn segments(&self) -> &[Segment] {
        &self.segs
    }

    /// À appeler ~10 fois/s avec la position AUDIO. Retourne où sauter, s'il y a lieu.
    /// On entre dans une fenêtre de ±0,15 s autour du début (le saut est quasi inaudible) et on
    /// ignore un segment dont il ne reste que 0,3 s.
    pub fn check(&mut self, t: f64, enabled: bool) -> Option<SkipDecision> {
        if !enabled {
            return None;
        }
        for (i, s) in self.segs.iter().enumerate() {
            if self.ignored.contains(&i) || self.done.contains(&i) {
                continue;
            }
            if t >= s.start - 0.15 && t < s.end - 0.3 {
                self.done.insert(i);
                self.last = Some(i);
                return Some(SkipDecision { to: s.end, category: s.category.clone(), from: t });
            }
        }
        None
    }

    /// L'utilisateur a re-sauté en arrière : on autorise à nouveau les segments situés après `t`.
    pub fn user_seeked(&mut self, t: f64) {
        self.done.retain(|&i| self.segs[i].end <= t);
    }

    /// « Annuler » après un saut : revient au début du segment et ne le saute plus.
    pub fn undo_last(&mut self) -> Option<f64> {
        let i = self.last.take()?;
        self.ignored.insert(i);
        Some(self.segs[i].start)
    }
}

// ─────────────────────────── réseau ───────────────────────────

#[cfg(feature = "net")]
pub use net::fetch_segments;

#[cfg(feature = "net")]
mod net {
    use super::{merge, Segment};
    use serde::Deserialize;
    use sha2::{Digest, Sha256};

    #[derive(Deserialize)]
    struct Entry {
        #[serde(rename = "videoID")]
        video_id: String,
        segments: Vec<Seg>,
    }
    #[derive(Deserialize)]
    struct Seg {
        segment: [f64; 2],
        category: String,
        #[serde(rename = "actionType", default)]
        action_type: String,
    }

    pub fn hash_prefix(video_id: &str) -> String {
        let d = Sha256::digest(video_id.as_bytes());
        d.iter().take(2).map(|b| format!("{b:02x}")).collect() // 4 premiers caractères hex
    }

    /// Interroge l'API par préfixe de hash (le serveur ne voit pas l'identifiant exact de la vidéo).
    /// `base` : « https://sponsor.ajay.app » ou un miroir.
    pub fn fetch_segments(base: &str, video_id: &str, categories: &[&str]) -> Result<Vec<Segment>, String> {
        let cats = format!("[{}]", categories.iter().map(|c| format!("\"{c}\"")).collect::<Vec<_>>().join(","));
        let client = reqwest::blocking::Client::builder().timeout(std::time::Duration::from_secs(8)).user_agent("Aurex/0.1").build().map_err(|e| e.to_string())?;
        let resp = client
            .get(format!("{}/api/skipSegments/{}", base.trim_end_matches('/'), hash_prefix(video_id)))
            .query(&[("categories", cats.as_str()), ("actionTypes", "[\"skip\"]")])
            .send()
            .map_err(|e| e.to_string())?;
        if resp.status().as_u16() == 404 {
            return Ok(vec![]); // aucun segment connu
        }
        if !resp.status().is_success() {
            return Err(format!("SponsorBlock HTTP {}", resp.status()));
        }
        let entries: Vec<Entry> = resp.json().map_err(|e| e.to_string())?;
        let segs = entries
            .into_iter()
            .filter(|e| e.video_id == video_id)
            .flat_map(|e| e.segments)
            .filter(|s| s.action_type.is_empty() || s.action_type == "skip")
            .map(|s| Segment { start: s.segment[0], end: s.segment[1], category: s.category })
            .collect();
        Ok(merge(segs))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seg(a: f64, b: f64, c: &str) -> Segment {
        Segment { start: a, end: b, category: c.into() }
    }

    #[test]
    fn merges_overlaps_and_drops_garbage() {
        let m = merge(vec![seg(10.0, 20.0, "sponsor"), seg(19.0, 30.0, "selfpromo"), seg(40.0, 40.0, "x"), seg(50.0, 55.0, "intro")]);
        assert_eq!(m.len(), 2);
        assert_eq!((m[0].start, m[0].end), (10.0, 30.0));
    }

    #[test]
    fn skips_once_and_respects_toggle() {
        let mut s = Skipper::new(vec![seg(10.0, 20.0, "sponsor")]);
        assert_eq!(s.check(9.0, true), None);
        assert_eq!(s.check(10.1, false), None); // désactivé à la volée
        let d = s.check(10.1, true).unwrap();
        assert_eq!((d.to, d.category.as_str()), (20.0, "sponsor"));
        assert_eq!(s.check(10.2, true), None); // déjà sauté : pas de boucle
    }

    #[test]
    fn ignores_when_almost_over_and_allows_after_user_seek_back() {
        let mut s = Skipper::new(vec![seg(10.0, 20.0, "sponsor")]);
        assert_eq!(s.check(19.8, true), None); // il reste 0,2 s
        assert!(s.check(12.0, true).is_some());
        s.user_seeked(5.0); // retour en arrière
        assert!(s.check(10.5, true).is_some());
    }

    #[test]
    fn undo_returns_to_start_and_never_skips_again() {
        let mut s = Skipper::new(vec![seg(10.0, 20.0, "sponsor")]);
        s.check(10.0, true).unwrap();
        assert_eq!(s.undo_last(), Some(10.0));
        s.user_seeked(0.0);
        assert_eq!(s.check(11.0, true), None);
    }
}
