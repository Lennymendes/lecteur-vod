//! Aurex — shell Tauri 2.
//!
//! Fenêtre unique transparente. Sous la webview : une surface native (HWND / NSView) qui reçoit la
//! vidéo de libmpv. L'UI React réserve un « trou » (`VideoSlot`) et en envoie le rectangle via
//! `video_set_rect`.

mod commands;
mod events;
mod external;
mod log;
mod sidecar;
mod state;
mod surface;
mod video;

use raw_window_handle::HasWindowHandle;
use state::AppState;
use std::sync::Arc;
use tauri::menu::{Menu, MenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::Manager;

/// macOS : indique au chargeur Vulkan le pilote MoltenVK embarqué dans le bundle (avant toute init de mpv).
fn setup_vulkan_icd() {
    if !cfg!(target_os = "macos") {
        return;
    }
    if let Some(icd) = std::env::current_exe()
        .ok()
        .and_then(|e| e.parent().map(|d| d.join("../Resources/vulkan/MoltenVK_icd.json")))
        .filter(|p| p.exists())
    {
        std::env::set_var("VK_DRIVER_FILES", &icd);
        std::env::set_var("VK_ICD_FILENAMES", &icd);
        log::log(format!("vulkan: ICD = {}", icd.display()));
    } else {
        log::log("vulkan: ICD MoltenVK introuvable dans le bundle");
    }
}

/// ffmpeg embarqué (OpenSSL) vérifie les certificats TLS mais cherche son magasin dans le préfixe
/// Homebrew de la machine de build. On fournit `cacert.pem` (bundle Mozilla) via SSL_CERT_FILE,
/// hérité par les processus ffmpeg lancés ensuite.
fn setup_ca_bundle() {
    let Some(dir) = std::env::current_exe().ok().and_then(|e| e.parent().map(|d| d.to_path_buf())) else { return };
    let cands = [
        dir.join("../Resources/cacert.pem"),
        dir.join("cacert.pem"),
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("resources/cacert.pem"),
    ];
    match cands.iter().find(|p| p.metadata().map(|m| m.len() > 1000).unwrap_or(false)) {
        Some(p) => {
            std::env::set_var("SSL_CERT_FILE", p);
            log::log(format!("tls: SSL_CERT_FILE = {}", p.display()));
        }
        None => log::log("tls: cacert.pem introuvable (ffmpeg risque d'échouer sur HTTPS)"),
    }
}

pub fn run() {
    setup_vulkan_icd();
    setup_ca_bundle();
    tauri::Builder::default()
        .manage(AppState::default())
        .invoke_handler(tauri::generate_handler![
            commands::yt_play,
            commands::twitch_play,
            commands::player_pause,
            commands::player_seek,
            commands::player_stop,
            commands::set_adblock,
            commands::set_sponsorblock,
            commands::sponsor_undo,
            commands::dsp_load_plugin,
            commands::dsp_unload_plugin,
            commands::dsp_set_bypass,
            commands::video_set_rect,
            commands::app_info,
            commands::export_logs,
        ])
        .setup(|app| {
            // Thread principal : création de la surface native sous la webview, puis de libmpv dessus.
            let win = app.get_webview_window("main").ok_or("fenêtre principale introuvable")?;
            let st = app.state::<AppState>();
            match win.window_handle() {
                Ok(h) => match surface::Surface::create(h.as_raw()) {
                    Ok(s) => {
                        let wid = s.wid();
                        log::log(format!("surface créée, wid={wid:#x}"));
                        *st.surface.lock().unwrap() = Some(s);
                        match video::Video::new(wid) {
                            Ok(v) => *st.video.lock().unwrap() = Some(Arc::new(v)),
                            Err(e) => {
                                log::log(format!("vidéo indisponible : {e}"));
                                eprintln!("[aurex] vidéo indisponible : {e}")
                            }
                        }
                    }
                    Err(e) => {
                        log::log(format!("surface indisponible : {e}"));
                        eprintln!("[aurex] surface vidéo indisponible : {e}")
                    }
                },
                Err(e) => eprintln!("[aurex] handle de fenêtre : {e}"),
            }

            // Icône de zone de notification.
            let show = MenuItem::with_id(app, "show", "Afficher Aurex", true, None::<&str>)?;
            let quit = MenuItem::with_id(app, "quit", "Quitter", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&show, &quit])?;
            let mut tray = TrayIconBuilder::new().menu(&menu).on_menu_event(|app, ev| match ev.id.as_ref() {
                "show" => {
                    if let Some(w) = app.get_webview_window("main") {
                        let _ = w.show();
                        let _ = w.set_focus();
                    }
                }
                "quit" => app.exit(0),
                _ => {}
            });
            if let Some(icon) = app.default_window_icon() {
                tray = tray.icon(icon.clone());
            }
            tray.build(app)?;
            Ok(())
        })
        .on_window_event(|win, ev| {
            // Fermeture : relâcher le DAC proprement (mode exclusif) avant de quitter.
            if let tauri::WindowEvent::CloseRequested { .. } = ev {
                let st = win.app_handle().state::<AppState>();
                st.stop_all(true);
            }
        })
        .run(tauri::generate_context!())
        .expect("erreur au lancement d'Aurex");
}
