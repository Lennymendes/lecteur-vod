//! Analyse de playlists HLS Twitch + filtre live qui retire les segments publicitaires SSAI
//! (Server-Side Ad Insertion) en gardant une numérotation stable pour le lecteur.
//!
//! Signaux de pub reconnus (observés dans les outils communautaires, voir ARCHITECTURE.md) :
//!   * `#EXT-X-DATERANGE` avec CLASS `twitch-stitched-ad` / `twitch-maf-ad`, ID `stitched-ad-*`,
//!     ou un attribut `X-TV-TWITCH-AD-*`  -> segments dont le PROGRAM-DATE-TIME tombe dans [START, START+durée[
//!   * `#EXT-X-CUE-OUT` … `#EXT-X-CUE-IN`
//!   * titre `EXTINF` contenant « Amazon » (ancien signal)
//! Le DATERANGE `twitch-trigger` est présent en permanence : ce n'est PAS une pub.
//!
//! Twitch peut changer ces marqueurs à tout moment : le module est isolé et testé par fixtures
//! pour qu'une adaptation tienne en quelques lignes.

use std::collections::VecDeque;

// ───────────────────────── utilitaires ─────────────────────────

/// `KEY=VALUE,KEY="va,lue"` -> paires (guillemets retirés).
pub fn parse_attrs(s: &str) -> Vec<(String, String)> {
    let b: Vec<char> = s.chars().collect();
    let (mut out, mut i) = (Vec::new(), 0);
    while i < b.len() {
        let ks = i;
        while i < b.len() && b[i] != '=' {
            i += 1;
        }
        if i >= b.len() {
            break;
        }
        let key: String = b[ks..i].iter().collect();
        i += 1;
        let val: String;
        if i < b.len() && b[i] == '"' {
            i += 1;
            let vs = i;
            while i < b.len() && b[i] != '"' {
                i += 1;
            }
            val = b[vs..i].iter().collect();
            i += 1;
        } else {
            let vs = i;
            while i < b.len() && b[i] != ',' {
                i += 1;
            }
            val = b[vs..i].iter().collect();
        }
        if i < b.len() && b[i] == ',' {
            i += 1;
        }
        out.push((key.trim().to_string(), val));
    }
    out
}

fn attr<'a>(attrs: &'a [(String, String)], key: &str) -> Option<&'a str> {
    attrs.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str())
}

fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = (if y >= 0 { y } else { y - 399 }) / 400;
    let yoe = y - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// RFC 3339 (`2026-11-18T17:00:00.123Z`) -> millisecondes Unix.
pub fn parse_rfc3339_ms(s: &str) -> Option<i64> {
    let (date, rest) = s.trim().split_once('T')?;
    let mut d = date.split('-');
    let (y, m, day): (i64, i64, i64) = (d.next()?.parse().ok()?, d.next()?.parse().ok()?, d.next()?.parse().ok()?);
    let (time, off_min) = if let Some(t) = rest.strip_suffix('Z') {
        (t, 0)
    } else if let Some(p) = rest.rfind(['+', '-']) {
        let (t, o) = rest.split_at(p);
        let sign = if o.starts_with('-') { -1 } else { 1 };
        let (hh, mm) = o[1..].split_once(':')?;
        (t, sign * (hh.parse::<i64>().ok()? * 60 + mm.parse::<i64>().ok()?))
    } else {
        (rest, 0)
    };
    let mut t = time.split(':');
    let (hh, mm): (i64, i64) = (t.next()?.parse().ok()?, t.next()?.parse().ok()?);
    let ss = t.next()?;
    let (sec, ms) = match ss.split_once('.') {
        Some((a, f)) => {
            let mut f3: String = f.chars().take(3).collect();
            while f3.len() < 3 {
                f3.push('0');
            }
            (a.parse::<i64>().ok()?, f3.parse::<i64>().ok()?)
        }
        None => (ss.parse::<i64>().ok()?, 0),
    };
    Some((days_from_civil(y, m, day) * 86_400 + hh * 3600 + mm * 60 + sec - off_min * 60) * 1000 + ms)
}

// ───────────────────────── playlist média ─────────────────────────

#[derive(Debug, Clone)]
pub struct Segment {
    /// Tags qui précèdent l'URI (EXTINF, PROGRAM-DATE-TIME…), sans DISCONTINUITY.
    pub tags: Vec<String>,
    pub uri: String,
    pub duration: f64,
    pub pdt_ms: Option<i64>,
    pub seq: u64,
    pub discontinuity: bool,
    pub is_ad: bool,
}

#[derive(Debug, Clone, Default)]
pub struct Parsed {
    pub header: Vec<String>,
    pub media_seq: u64,
    pub segments: Vec<Segment>,
    /// Signaux de pub rencontrés (diagnostic / télémétrie).
    pub ad_signals: Vec<&'static str>,
}

impl Parsed {
    pub fn ad_segments(&self) -> usize {
        self.segments.iter().filter(|s| s.is_ad).count()
    }
    /// Une pub est annoncée ou en cours dans cette playlist.
    pub fn has_ads(&self) -> bool {
        !self.ad_signals.is_empty() || self.ad_segments() > 0
    }
}

fn is_ad_daterange(a: &[(String, String)]) -> Option<&'static str> {
    let class = attr(a, "CLASS").unwrap_or("");
    let id = attr(a, "ID").unwrap_or("");
    if class.contains("stitched-ad") {
        Some("daterange:twitch-stitched-ad")
    } else if class.contains("maf-ad") {
        Some("daterange:twitch-maf-ad")
    } else if id.starts_with("stitched-ad") {
        Some("daterange:id-stitched-ad")
    } else if a.iter().any(|(k, _)| k.starts_with("X-TV-TWITCH-AD")) {
        Some("daterange:x-tv-twitch-ad")
    } else {
        None // dont `twitch-trigger` : permanent, pas une pub
    }
}

pub fn parse_media(text: &str) -> Parsed {
    let mut p = Parsed::default();
    let mut pending: Vec<String> = Vec::new();
    let (mut dur, mut pdt, mut disc, mut in_cue, mut legacy_title_ad) = (0.0f64, None, false, false, false);
    let mut ranges: Vec<(i64, i64)> = Vec::new();
    let mut seen_segment = false;
    let mut cue_flags: Vec<bool> = Vec::new();

    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() {
            continue;
        }
        if let Some(rest) = line.strip_prefix("#EXT-X-MEDIA-SEQUENCE:") {
            p.media_seq = rest.trim().parse().unwrap_or(0);
        } else if let Some(rest) = line.strip_prefix("#EXT-X-DATERANGE:") {
            let a = parse_attrs(rest);
            if let Some(sig) = is_ad_daterange(&a) {
                if !p.ad_signals.contains(&sig) {
                    p.ad_signals.push(sig);
                }
                if let Some(start) = attr(&a, "START-DATE").and_then(parse_rfc3339_ms) {
                    let end = attr(&a, "END-DATE")
                        .and_then(parse_rfc3339_ms)
                        .or_else(|| {
                            attr(&a, "DURATION")
                                .or_else(|| attr(&a, "PLANNED-DURATION"))
                                .and_then(|d| d.parse::<f64>().ok())
                                .map(|d| start + (d * 1000.0) as i64)
                        })
                        .unwrap_or(start + 30_000); // durée inconnue : 30 s
                    ranges.push((start, end));
                }
            }
            // les DATERANGE ne sont jamais recopiés : inutiles pour la lecture
        } else if line.starts_with("#EXT-X-CUE-OUT") {
            in_cue = true;
            if !p.ad_signals.contains(&"cue-out") {
                p.ad_signals.push("cue-out");
            }
        } else if line.starts_with("#EXT-X-CUE-IN") {
            in_cue = false;
        } else if line == "#EXT-X-DISCONTINUITY" {
            disc = true;
        } else if line == "#EXT-X-ENDLIST" {
            // flux live : ignoré
        } else if let Some(rest) = line.strip_prefix("#EXTINF:") {
            let (d, title) = rest.split_once(',').unwrap_or((rest, ""));
            dur = d.trim().parse().unwrap_or(0.0);
            legacy_title_ad = title.contains("Amazon");
            pending.push(line.to_string());
        } else if let Some(rest) = line.strip_prefix("#EXT-X-PROGRAM-DATE-TIME:") {
            pdt = parse_rfc3339_ms(rest);
            pending.push(line.to_string());
        } else if line.starts_with('#') {
            // Avant le 1er segment : en-tête (EXTM3U, VERSION, TARGETDURATION…).
            // Ensuite : tag rattaché au segment suivant. Les tags orphelins de fin
            // (ex. EXT-X-TWITCH-PREFETCH) sont volontairement abandonnés.
            if seen_segment {
                pending.push(line.to_string());
            } else {
                p.header.push(line.to_string());
            }
        } else {
            let seq = p.media_seq + p.segments.len() as u64;
            p.segments.push(Segment {
                tags: std::mem::take(&mut pending),
                uri: line.to_string(),
                duration: dur,
                pdt_ms: pdt.take(),
                seq,
                discontinuity: std::mem::take(&mut disc),
                is_ad: false,
            });
            cue_flags.push(in_cue || legacy_title_ad);
            if in_cue && !p.ad_signals.contains(&"cue-out") {
                p.ad_signals.push("cue-out");
            }
            if legacy_title_ad && !p.ad_signals.contains(&"extinf:amazon") {
                p.ad_signals.push("extinf:amazon");
            }
            legacy_title_ad = false;
            seen_segment = true;
        }
    }
    for (seg, cue) in p.segments.iter_mut().zip(cue_flags) {
        let in_range = seg.pdt_ms.is_some_and(|t| ranges.iter().any(|&(s, e)| s <= t && t < e));
        seg.is_ad = cue || in_range;
    }
    p
}

// ───────────────────────── filtre live ─────────────────────────

#[derive(Debug, Clone)]
struct OutSeg {
    disc: bool,
    tags: Vec<String>,
    uri: String,
    key: i64,
}

#[derive(Debug, Clone)]
pub struct Filtered {
    pub playlist: String,
    /// Segments publicitaires retirés lors de CETTE mise à jour.
    pub removed: usize,
    /// Une pub est annoncée/en cours dans la source (même si le filtre l'a retirée).
    pub ad_active: bool,
}

/// Réécrit une playlist live en retirant les segments pub.
/// Identité d'un segment = PROGRAM-DATE-TIME (sinon n° de séquence) : indépendant de la
/// numérotation de la source, donc compatible avec un changement de session/backup en cours de route.
/// Sortie : MEDIA-SEQUENCE propre et monotone + `#EXT-X-DISCONTINUITY` après chaque trou.
pub struct LivePlaylistFilter {
    window: usize,
    last_key: Option<i64>,
    kept: VecDeque<OutSeg>,
    base_seq: u64,
    disc_pending: bool,
    header: Vec<String>,
    strip: bool,
}

impl LivePlaylistFilter {
    pub fn new(window: usize) -> Self {
        Self { window: window.max(3), last_key: None, kept: VecDeque::new(), base_seq: 0, disc_pending: false, header: Vec::new(), strip: true }
    }

    /// Active/désactive le retrait des pubs À LA VOLÉE. Désactivé, les segments pub passent
    /// (la numérotation reste stable, donc le lecteur n'a pas à être relancé).
    pub fn set_strip(&mut self, on: bool) {
        self.strip = on;
    }

    pub fn update(&mut self, text: &str) -> Filtered {
        let p = parse_media(text);
        if !p.header.is_empty() {
            self.header = p.header.clone();
        }
        let mut removed = 0;
        for s in &p.segments {
            let key = s.pdt_ms.unwrap_or(s.seq as i64);
            if self.last_key.is_some_and(|k| key <= k) {
                continue; // déjà vu
            }
            self.last_key = Some(key);
            if s.is_ad && self.strip {
                removed += 1;
                self.disc_pending = true;
                continue;
            }
            self.kept.push_back(OutSeg {
                disc: s.discontinuity || std::mem::take(&mut self.disc_pending),
                tags: s.tags.clone(),
                uri: s.uri.clone(),
                key,
            });
            while self.kept.len() > self.window {
                self.kept.pop_front();
                self.base_seq += 1;
            }
        }
        Filtered { playlist: self.render(), removed, ad_active: p.has_ads() }
    }

    fn render(&self) -> String {
        let mut o = String::new();
        if self.header.is_empty() {
            o.push_str("#EXTM3U\n#EXT-X-VERSION:3\n#EXT-X-TARGETDURATION:6\n");
        } else {
            for h in &self.header {
                o.push_str(h);
                o.push('\n');
            }
        }
        o.push_str(&format!("#EXT-X-MEDIA-SEQUENCE:{}\n", self.base_seq));
        for s in &self.kept {
            if s.disc {
                o.push_str("#EXT-X-DISCONTINUITY\n");
            }
            for t in &s.tags {
                o.push_str(t);
                o.push('\n');
            }
            o.push_str(&s.uri);
            o.push('\n');
        }
        o
    }

    /// Clé du dernier segment propre émis (utile pour les diagnostics).
    pub fn last_kept_key(&self) -> Option<i64> {
        self.kept.back().map(|s| s.key)
    }
}

// ───────────────────────── playlist maître ─────────────────────────

#[derive(Debug, Clone)]
pub struct Variant {
    pub url: String,
    pub bandwidth: u64,
    /// Groupe VIDEO (`chunked` = qualité source, `720p60`, `audio_only`…).
    pub video: String,
    pub name: String,
}

pub fn parse_master(text: &str) -> Vec<Variant> {
    let mut names: Vec<(String, String)> = Vec::new(); // GROUP-ID -> NAME
    let mut out = Vec::new();
    let mut pending: Option<Vec<(String, String)>> = None;
    for line in text.lines().map(str::trim) {
        if let Some(r) = line.strip_prefix("#EXT-X-MEDIA:") {
            let a = parse_attrs(r);
            if let (Some(g), Some(n)) = (attr(&a, "GROUP-ID"), attr(&a, "NAME")) {
                names.push((g.to_string(), n.to_string()));
            }
        } else if let Some(r) = line.strip_prefix("#EXT-X-STREAM-INF:") {
            pending = Some(parse_attrs(r));
        } else if !line.starts_with('#') && !line.is_empty() {
            if let Some(a) = pending.take() {
                let video = attr(&a, "VIDEO").unwrap_or("").to_string();
                let name = names.iter().find(|(g, _)| *g == video).map(|(_, n)| n.clone()).unwrap_or_default();
                out.push(Variant { url: line.to_string(), bandwidth: attr(&a, "BANDWIDTH").and_then(|b| b.parse().ok()).unwrap_or(0), video, name });
            }
        }
    }
    out
}

/// `quality` : "best" | "chunked" | "720p60" | "audio_only" | sous-chaîne du nom ("1080p")…
pub fn pick_variant<'a>(v: &'a [Variant], quality: &str) -> Option<&'a Variant> {
    let non_audio = || v.iter().filter(|x| x.video != "audio_only").max_by_key(|x| x.bandwidth);
    match quality {
        "best" | "source" => v.iter().find(|x| x.video == "chunked").or_else(non_audio),
        q => v.iter().find(|x| x.video == q || x.name.contains(q)).or_else(non_audio),
    }
}

#[cfg(test)]
mod tests;
