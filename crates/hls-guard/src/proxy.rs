//! Mini-serveur HTTP local qui sert la playlist nettoyée à libmpv.
//! - écoute 127.0.0.1 sur un port éphémère
//! - secret aléatoire dans le chemin + contrôle de l'en-tête Host (anti DNS-rebinding / autres apps locales)

use crate::session::{Guard, GuardEvent};
use axum::extract::{Path, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::Router;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::sync::{broadcast, Mutex, RwLock};

struct AppState {
    secret: String,
    port: u16,
    sessions: RwLock<HashMap<String, Arc<Mutex<Guard>>>>,
}

#[derive(Clone)]
pub struct ProxyHandle {
    st: Arc<AppState>,
}

impl ProxyHandle {
    /// Enregistre une session ; retourne l'URL à donner à `mpv loadfile` et le flux d'événements
    /// (backup de flux, coupure pub, retour à la normale).
    pub async fn serve(&self, channel: &str, guard: Guard) -> (String, broadcast::Receiver<GuardEvent>) {
        let key = channel.to_lowercase();
        let events = guard.subscribe();
        self.st.sessions.write().await.insert(key.clone(), Arc::new(Mutex::new(guard)));
        (format!("http://127.0.0.1:{}/s/{}/{}.m3u8", self.st.port, self.st.secret, key), events)
    }

    /// Interrupteur anti-pub à la volée, sans relancer le lecteur.
    pub async fn set_enabled(&self, channel: &str, on: bool) {
        let g = self.st.sessions.read().await.get(&channel.to_lowercase()).cloned();
        if let Some(g) = g {
            g.lock().await.set_enabled(on);
        }
    }

    pub async fn stop(&self, channel: &str) {
        self.st.sessions.write().await.remove(&channel.to_lowercase());
    }
}

pub async fn spawn_proxy() -> std::io::Result<ProxyHandle> {
    let listener = tokio::net::TcpListener::bind(SocketAddr::from(([127, 0, 0, 1], 0))).await?;
    let port = listener.local_addr()?.port();
    let secret: String = (0..24).map(|_| format!("{:x}", rand::random::<u8>() % 16)).collect();
    let st = Arc::new(AppState { secret, port, sessions: RwLock::new(HashMap::new()) });
    let app = Router::new().route("/s/:secret/:file", get(playlist)).with_state(st.clone());
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    Ok(ProxyHandle { st })
}

async fn playlist(State(st): State<Arc<AppState>>, Path((secret, file)): Path<(String, String)>, headers: HeaderMap) -> Response {
    let host_ok = headers.get(header::HOST).and_then(|h| h.to_str().ok()).is_some_and(|h| h == format!("127.0.0.1:{}", st.port));
    if !host_ok || secret != st.secret {
        return StatusCode::FORBIDDEN.into_response();
    }
    let chan = file.trim_end_matches(".m3u8").to_lowercase();
    let guard = st.sessions.read().await.get(&chan).cloned();
    let Some(guard) = guard else { return StatusCode::NOT_FOUND.into_response() };
    let mut g = guard.lock().await;
    match g.next_playlist().await {
        Ok(text) => (
            [(header::CONTENT_TYPE, "application/vnd.apple.mpegurl"), (header::CACHE_CONTROL, "no-store")],
            text,
        )
            .into_response(),
        Err(e) => (StatusCode::BAD_GATEWAY, e.to_string()).into_response(),
    }
}
