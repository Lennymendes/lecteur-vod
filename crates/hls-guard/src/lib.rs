//! Résolution Twitch (GQL + usher), anti-pub SSAI et proxy HLS local.
//!
//! Flux : `Guard` obtient une playlist média, la nettoie (backup player-type, sinon retrait des
//! segments pub), et `proxy` la sert à libmpv sur `http://127.0.0.1:<port>/s/<secret>/<chan>.m3u8`.
//! Les segments (.ts) ne transitent PAS par le proxy : le lecteur les télécharge directement au CDN.

mod proxy;
mod session;
mod twitch;

pub use proxy::{spawn_proxy, ProxyHandle};
pub use session::{Guard, GuardEvent};
pub use twitch::{GuardError, TwitchHls};
