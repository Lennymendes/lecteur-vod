//! Politique anti-pub, par ordre de préférence :
//!   1. Playlist propre                      -> passage tel quel.
//!   2. Pub détectée -> « backup » : redemander jeton + usher avec un autre `playerType`
//!      et prendre la première playlist sans marqueur pub (technique des outils communautaires).
//!   3. Aucun backup propre                  -> retirer les segments pub (trou signalé par
//!      DISCONTINUITY) ; le lecteur attend la fin de la coupure. Jamais d'écran violet silencieux :
//!      un événement est émis pour que l'UI affiche « coupure pub ≈ 30 s ».
//! Sur backup, on sonde périodiquement le flux principal pour revenir dessus (meilleure qualité).

use crate::twitch::{GuardError, TwitchHls};
use hls_playlist::{parse_media, LivePlaylistFilter};
use std::time::{Duration, Instant};
use tokio::sync::broadcast;

#[derive(Clone, Debug)]
pub enum GuardEvent {
    /// Pub contournée via un autre flux.
    BackupStream { player_type: String },
    /// Pas de flux propre : segments pub retirés, lecture suspendue ≈ `segments` × 2 s.
    Stripped { segments: usize },
    /// Retour au flux normal.
    Clean,
    Error(String),
}

pub struct Guard {
    tw: TwitchHls,
    channel: String,
    quality: String,
    /// Index 0 = flux principal. Les suivants sont des `playerType` de secours ;
    /// valeurs observées dans des outils communautaires, À AJUSTER par télémétrie.
    player_types: Vec<String>,
    active: usize,
    media_url: String,
    filter: LivePlaylistFilter,
    enabled: bool,
    in_break: bool,
    last_probe: Instant,
    last_out: Option<(Instant, String)>,
    events: broadcast::Sender<GuardEvent>,
}

impl Guard {
    pub async fn start(tw: TwitchHls, channel: &str, quality: &str) -> Result<Self, GuardError> {
        let player_types: Vec<String> = ["site", "embed", "autoplay", "picture-by-picture"].iter().map(|s| s.to_string()).collect();
        let media_url = tw.media_url(channel, &player_types[0], quality).await?;
        let (events, _) = broadcast::channel(16);
        Ok(Self {
            tw,
            channel: channel.to_string(),
            quality: quality.to_string(),
            player_types,
            active: 0,
            media_url,
            filter: LivePlaylistFilter::new(8),
            enabled: true,
            in_break: false,
            last_probe: Instant::now(),
            last_out: None,
            events,
        })
    }

    /// Active/désactive l'anti-pub à la volée (bouton de l'UI). Désactivé : la playlist passe
    /// telle quelle (pubs incluses) mais avec la même numérotation, sans relancer le lecteur.
    pub fn set_enabled(&mut self, on: bool) {
        self.enabled = on;
        self.filter.set_strip(on);
        self.last_out = None; // appliquer immédiatement
    }

    pub fn subscribe(&self) -> broadcast::Receiver<GuardEvent> {
        self.events.subscribe()
    }

    /// Appelé à chaque rafraîchissement du lecteur (≈ toutes les 2 s). Mis en cache 400 ms.
    pub async fn next_playlist(&mut self) -> Result<String, GuardError> {
        if let Some((t, s)) = &self.last_out {
            if t.elapsed() < Duration::from_millis(400) {
                return Ok(s.clone());
            }
        }
        let out = self.compute().await?;
        self.last_out = Some((Instant::now(), out.clone()));
        Ok(out)
    }

    async fn fetch_active(&mut self) -> Result<String, GuardError> {
        match self.tw.fetch_text(&self.media_url).await {
            Err(GuardError::Expired) => {
                // jeton périmé : on en redemande un pour le même playerType, une seule fois
                self.media_url = self.tw.media_url(&self.channel, &self.player_types[self.active], &self.quality).await?;
                self.tw.fetch_text(&self.media_url).await
            }
            r => r,
        }
    }

    async fn try_type(&self, idx: usize) -> Option<(String, String)> {
        let url = self.tw.media_url(&self.channel, &self.player_types[idx], &self.quality).await.ok()?;
        let text = self.tw.fetch_text(&url).await.ok()?;
        (!parse_media(&text).has_ads()).then_some((url, text))
    }

    async fn compute(&mut self) -> Result<String, GuardError> {
        let text = self.fetch_active().await?;

        if !self.enabled {
            return Ok(self.filter.update(&text).playlist);
        }

        if !parse_media(&text).has_ads() {
            // Sur un flux de secours : retenter le principal toutes les 20 s.
            if self.active != 0 && self.last_probe.elapsed() > Duration::from_secs(20) {
                self.last_probe = Instant::now();
                if let Some((url, t)) = self.try_type(0).await {
                    self.active = 0;
                    self.media_url = url;
                    self.mark_clean();
                    return Ok(self.filter.update(&t).playlist);
                }
            }
            self.mark_clean();
            return Ok(self.filter.update(&text).playlist);
        }

        // Pub détectée : chercher un flux de secours propre.
        for idx in 0..self.player_types.len() {
            if idx == self.active {
                continue;
            }
            if let Some((url, t)) = self.try_type(idx).await {
                self.active = idx;
                self.media_url = url;
                self.last_probe = Instant::now();
                self.in_break = true;
                let _ = self.events.send(GuardEvent::BackupStream { player_type: self.player_types[idx].clone() });
                return Ok(self.filter.update(&t).playlist);
            }
        }

        // Aucun flux propre : on retire les segments pub.
        let f = self.filter.update(&text);
        if !self.in_break || f.removed > 0 {
            let _ = self.events.send(GuardEvent::Stripped { segments: f.removed });
        }
        self.in_break = true;
        Ok(f.playlist)
    }

    fn mark_clean(&mut self) {
        if self.in_break {
            self.in_break = false;
            let _ = self.events.send(GuardEvent::Clean);
        }
    }
}
