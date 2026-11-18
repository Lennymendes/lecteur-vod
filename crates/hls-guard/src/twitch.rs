//! Obtention du M3U8 Twitch SANS le lecteur web :
//!   1. GQL `PlaybackAccessToken` -> (token, signature) à durée de vie courte
//!   2. usher.ttvnw.net -> playlist maître (variantes de qualité)
//!   3. choix d'une variante -> playlist média live
//!
//! ⚠ API non documentée (celle du site web). Elle peut changer sans préavis, exiger des en-têtes
//! d'intégrité, ou être limitée. Le module est isolé pour que l'adaptation reste locale.
//! Requête GQL en texte brut (pas de hash « persisted query » : Twitch les fait tourner).

use hls_playlist::{parse_master, pick_variant};
use serde_json::{json, Value};
use std::fmt;
use std::time::Duration;

const GQL_URL: &str = "https://gql.twitch.tv/gql";
const USHER_URL: &str = "https://usher.ttvnw.net/api/channel/hls/";
/// Client-ID public du site web Twitch (le même que celui d'outils comme Streamlink / twitch-hls-client).
const WEB_CLIENT_ID: &str = "kimne78kx3ncx6brgo4mv6wki5h1ko";

const TOKEN_QUERY: &str = r#"query PlaybackAccessToken_Template($login: String!, $isLive: Boolean!, $vodID: ID!, $isVod: Boolean!, $playerType: String!) {
  streamPlaybackAccessToken(channelName: $login, params: {platform: "web", playerBackend: "mediaplayer", playerType: $playerType}) @include(if: $isLive) { value signature }
  videoPlaybackAccessToken(id: $vodID, params: {platform: "web", playerBackend: "mediaplayer", playerType: $playerType}) @include(if: $isVod) { value signature }
}"#;

#[derive(Debug)]
pub enum GuardError {
    /// Chaîne hors ligne ou inexistante.
    NotLive,
    /// 401/403/410 : jeton expiré, à renouveler.
    Expired,
    Http(String),
    Parse(String),
}

impl fmt::Display for GuardError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            GuardError::NotLive => write!(f, "chaîne hors ligne"),
            GuardError::Expired => write!(f, "jeton expiré"),
            GuardError::Http(s) => write!(f, "HTTP : {s}"),
            GuardError::Parse(s) => write!(f, "réponse inattendue : {s}"),
        }
    }
}
impl std::error::Error for GuardError {}

impl From<reqwest::Error> for GuardError {
    fn from(e: reqwest::Error) -> Self {
        GuardError::Http(e.to_string())
    }
}

#[derive(Clone)]
pub struct TwitchHls {
    http: reqwest::Client,
    /// Jeton OAuth *du site web* (cookie `auth-token`) : c'est lui qui rend Turbo / abonnement
    /// effectifs côté pubs. Le jeton Helix de l'app (OAuth officiel) n'est pas équivalent.
    web_oauth: Option<String>,
    device_id: String,
}

impl TwitchHls {
    pub fn new(web_oauth: Option<String>) -> Result<Self, GuardError> {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(8))
            .user_agent("Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130 Safari/537.36")
            .build()?;
        let device_id: String = (0..32).map(|_| format!("{:x}", rand::random::<u8>() % 16)).collect();
        Ok(Self { http, web_oauth, device_id })
    }

    pub async fn access_token(&self, channel: &str, player_type: &str) -> Result<(String, String), GuardError> {
        let body = json!({
            "operationName": "PlaybackAccessToken_Template",
            "query": TOKEN_QUERY,
            "variables": { "isLive": true, "login": channel.to_lowercase(), "isVod": false, "vodID": "", "playerType": player_type }
        });
        let mut req = self.http.post(GQL_URL).header("Client-ID", WEB_CLIENT_ID).header("X-Device-Id", &self.device_id).json(&body);
        if let Some(t) = &self.web_oauth {
            req = req.header("Authorization", format!("OAuth {t}"));
        }
        let resp = req.send().await?;
        match resp.status().as_u16() {
            401 | 403 => return Err(GuardError::Expired),
            s if s >= 400 => return Err(GuardError::Http(format!("gql {s}"))),
            _ => {}
        }
        let v: Value = resp.json().await?;
        let t = &v["data"]["streamPlaybackAccessToken"];
        match (t["value"].as_str(), t["signature"].as_str()) {
            (Some(val), Some(sig)) => Ok((val.to_string(), sig.to_string())),
            _ => Err(GuardError::Parse(format!("pas de jeton (errors={})", v["errors"]))),
        }
    }

    pub async fn master(&self, channel: &str, token: &str, sig: &str) -> Result<String, GuardError> {
        let p = rand::random::<u32>().to_string();
        let url = url::Url::parse_with_params(
            &format!("{USHER_URL}{}.m3u8", channel.to_lowercase()),
            &[
                ("allow_source", "true"),
                ("allow_audio_only", "true"),
                ("player_backend", "mediaplayer"),
                ("platform", "web"),
                ("playlist_include_framerate", "true"),
                ("supported_codecs", "avc1"), // H.264 : décodage matériel partout
                ("p", p.as_str()),
                ("sig", sig),
                ("token", token),
            ],
        )
        .map_err(|e| GuardError::Parse(e.to_string()))?;
        let resp = self.http.get(url).send().await?;
        match resp.status().as_u16() {
            404 => Err(GuardError::NotLive),
            401 | 403 => Err(GuardError::Expired),
            s if s >= 400 => Err(GuardError::Http(format!("usher {s}"))),
            _ => Ok(resp.text().await?),
        }
    }

    /// URL de la playlist média de `quality` pour ce `player_type`.
    pub async fn media_url(&self, channel: &str, player_type: &str, quality: &str) -> Result<String, GuardError> {
        let (token, sig) = self.access_token(channel, player_type).await?;
        let master = self.master(channel, &token, &sig).await?;
        let variants = parse_master(&master);
        pick_variant(&variants, quality).map(|v| v.url.clone()).ok_or_else(|| GuardError::Parse("aucune variante".into()))
    }

    pub async fn fetch_text(&self, url: &str) -> Result<String, GuardError> {
        let resp = self.http.get(url).send().await?;
        match resp.status().as_u16() {
            401 | 403 | 410 => Err(GuardError::Expired),
            404 => Err(GuardError::NotLive),
            s if s >= 400 => Err(GuardError::Http(format!("{s}"))),
            _ => Ok(resp.text().await?),
        }
    }
}
