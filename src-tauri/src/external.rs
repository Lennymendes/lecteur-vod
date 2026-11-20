//! Lecture dans un lecteur externe (VLC, mpv, IINA ou commande personnalisée).
//! Twitch : on passe l'URL du proxy local → l'anti-pub reste actif. YouTube : URLs résolues par yt-dlp
//! (vidéo + piste audio séparée), avec les en-têtes HTTP fournis par yt-dlp (User-Agent, Referer) :
//! sans eux googlevideo.com répond souvent 403. Les arguments sont passés tels quels au processus
//! (pas de shell) : `?` et `&` des URL n'ont pas besoin d'échappement. L'audio n'est alors plus géré par Aurex (ni exclusif ni pont DSP).

use serde::Deserialize;
use std::path::PathBuf;
use std::process::{Command, Stdio};

#[derive(Deserialize, Clone, Debug)]
pub struct External {
    /// "vlc" | "mpv" | "iina" | "custom"
    pub player: String,
    /// Commande personnalisée ; jetons : {video} {audio} {title} {ua}
    pub custom: Option<String>,
}

fn first_existing(cands: &[&str], fallback: &str) -> PathBuf {
    cands.iter().map(PathBuf::from).find(|p| p.exists()).unwrap_or_else(|| PathBuf::from(fallback))
}

fn header<'a>(h: &'a [(String, String)], name: &str) -> Option<&'a str> {
    h.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
}

pub fn launch(ext: &External, video: &str, audio: Option<&str>, title: &str, headers: &[(String, String)]) -> Result<String, String> {
    let ua = header(headers, "user-agent");
    let referer = header(headers, "referer");
    let (bin, args, name): (PathBuf, Vec<String>, &str) = match ext.player.as_str() {
        "vlc" => {
            let bin = first_existing(
                &["/Applications/VLC.app/Contents/MacOS/VLC", r"C:\Program Files\VideoLAN\VLC\vlc.exe", r"C:\Program Files (x86)\VideoLAN\VLC\vlc.exe"],
                "vlc",
            );
            let mut a = vec![format!("--meta-title={title}")];
            if let Some(u) = ua {
                a.push(format!("--http-user-agent={u}"));
            }
            if let Some(r) = referer {
                a.push(format!("--http-referrer={r}"));
            }
            if let Some(au) = audio {
                a.push(format!("--input-slave={au}"));
            }
            a.push(video.to_string());
            (bin, a, "VLC")
        }
        "mpv" => {
            let bin = first_existing(&["/opt/homebrew/bin/mpv", "/usr/local/bin/mpv", "/Applications/mpv.app/Contents/MacOS/mpv"], "mpv");
            let mut a = vec![format!("--force-media-title={title}")];
            if let Some(u) = ua {
                a.push(format!("--user-agent={u}"));
            }
            if let Some(r) = referer {
                a.push(format!("--referrer={r}"));
            }
            if let Some(au) = audio {
                a.push(format!("--audio-file={au}"));
            }
            a.push(video.to_string());
            (bin, a, "mpv")
        }
        "iina" => {
            let bin = first_existing(&["/Applications/IINA.app/Contents/MacOS/iina-cli"], "iina-cli");
            let mut a = vec![format!("--mpv-force-media-title={title}")];
            if let Some(u) = ua {
                a.push(format!("--mpv-user-agent={u}"));
            }
            if let Some(r) = referer {
                a.push(format!("--mpv-referrer={r}"));
            }
            if let Some(au) = audio {
                a.push(format!("--mpv-audio-file={au}"));
            }
            a.push(video.to_string());
            (bin, a, "IINA")
        }
        "custom" => {
            let cmd = ext.custom.as_deref().unwrap_or("").trim();
            let mut parts = cmd.split_whitespace().map(|t| {
                t.replace("{video}", video).replace("{audio}", audio.unwrap_or("")).replace("{title}", title).replace("{ua}", ua.unwrap_or(""))
            });
            let bin = parts.next().ok_or("commande personnalisée vide (ex. : vlc {video} --input-slave={audio})")?;
            let mut a: Vec<String> = parts.filter(|t| !t.is_empty()).collect();
            if !cmd.contains("{video}") {
                a.push(video.to_string());
            }
            (PathBuf::from(bin), a, "lecteur personnalisé")
        }
        other => return Err(format!("lecteur inconnu : {other}")),
    };
    crate::log::log(format!("externe: {} {:?}", bin.display(), args.iter().map(|a| a.chars().take(80).collect::<String>()).collect::<Vec<_>>()));
    Command::new(&bin)
        .args(&args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("{name} introuvable ({}) : {e}. Installez-le ou choisissez « Commande personnalisée » dans les réglages.", bin.display()))?;
    Ok(name.to_string())
}
