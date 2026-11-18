# Aurex : état du projet (18 novembre 2026)

Résumé destiné à un autre assistant (Gemini) pour aider à organiser la suite. Dépôt : https://github.com/Lennymendes/lecteur-vod (branche `main`, seule source de vérité).

## 1. Le produit

Aurex est un lecteur de bureau **macOS + Windows** qui réunit **Twitch** et **YouTube** dans une seule application, pensé pour l'audiophile :

- **Twitch sans pub** : contournement des pubs insérées côté serveur (SSAI) dans le flux HLS.
- **YouTube natif** (façon yt-dlp, choix assumé hors CGU de l'API) avec **SponsorBlock** et sans pub.
- **Audio bit-perfect** : sortie exclusive (CoreAudio *hog mode* sur Mac, WASAPI Exclusive sur Windows), avec **changement automatique de fréquence** du DAC (44,1 / 48 kHz…) selon la source.
- **Pont DSP** : une API C stable pour brancher un futur traitement audio en C++ juste avant le DAC.
- Matériel de test visé : RME ADI-2 Pro, Merging Anubis, MacBook Pro (Apple Silicon).

## 2. Architecture technique

| Couche | Choix |
|---|---|
| Cœur | Rust (workspace Cargo) |
| Fenêtre / UI | Tauri 2 + React/TypeScript/Vite, fenêtre transparente |
| Vidéo | libmpv (crate `libmpv2`), rendue dans une vue native **sous** la webview (NSView sur Mac, HWND enfant sur Windows) |
| Extraction YouTube | `yt-dlp` embarqué (sidecar) + **Deno** (moteur JS exigé par yt-dlp depuis fin 2025) |
| Décodage audio YouTube | `ffmpeg` embarqué → PCM float32 → moteur audio maison |
| CI / builds | GitHub Actions : tests sur 3 OS, installeurs macOS (.zip) et Windows (.exe / .msi) |

**Crates Rust :**
- `audio-core` : types communs, pont DSP (ABI C v1, `include/aurex_dsp.h`, chargement de plugin `.dylib/.dll`).
- `audio-engine` : sortie exclusive CoreAudio / WASAPI, réouverture du DAC si la fréquence change, badge « bit-perfect ».
- `hls-playlist` : parseur de playlists HLS, détection des pubs Twitch.
- `hls-guard` : jeton Twitch (GQL) → playlist → filtrage des pubs (flux de secours ou retrait des segments) → **proxy local** `127.0.0.1` lu par mpv.
- `yt-native` : appel yt-dlp, choix des formats (audio/vidéo séparés), décodage ffmpeg, synchro vidéo par asservissement de vitesse mpv, SponsorBlock (API par préfixe de hash).
- `src-tauri` (`aurex`) : commandes IPC, état, surface vidéo native, journaux, lecteur externe.

**Deux chemins de lecture :**
1. **YouTube vidéo (chemin natif)** : audio → `audio-engine` (exclusif, pont DSP actif) ; vidéo seule → mpv, calée sur l'horloge audio.
2. **Twitch et directs YouTube** : mpv lit tout, avec **sa propre** sortie audio exclusive (hors pont DSP pour l'instant).

Une seule chose tient la sortie exclusive à la fois (`stop_all` libère le DAC avant que mpv ouvre la sienne).

## 3. Ce qui est fait et vérifié

- Dépôt GitHub propre, CI verte sur macOS (arm64) et Windows (x64) ; installeurs téléchargeables depuis l'onglet *Actions*.
- App macOS autonome : libmpv, ffmpeg, MoltenVK (pilote Vulkan) et Deno embarqués ; plus besoin de Homebrew.
- **Twitch testé sur Mac : image + son OK**, anti-pub actif par défaut, badge « Exclusif float32 48 kHz ».
- **Directs YouTube testés : OK**.
- Bouton « Exporter les journaux » (fichier `Aurex-logs.txt` sur le Bureau) pour le diagnostic.

**Problèmes résolus en chemin :** signature/quarantaine macOS (« app endommagée »), bibliothèques Homebrew manquantes, `LC_RPATH` dupliqués, absence de pilote Vulkan (pas d'image), erreurs de compilation Windows (WASAPI, `mpv.lib`).

## 4. Livré dans la dernière version (à tester)

- **Vidéos YouTube** : elles échouaient faute de moteur JavaScript pour yt-dlp ; Deno est maintenant embarqué.
- **Qualité** : sélecteur à côté du bouton « Lire » (Twitch : Source → 160p ou audio seul ; YouTube : résolution max 360p → 4K). Changer la qualité relance le flux en cours.
- **Réglages** (icône ⚙︎) : choix du lecteur (Aurex intégré, VLC, mpv, IINA, commande personnalisée), plugin DSP, export des journaux. Avec un lecteur externe, Twitch passe toujours par le proxy anti-pub.
- **Lecteur plus propre** : fond noir derrière la vidéo (plus de bureau visible sur les côtés), interface opaque.

## 5. Pas encore vérifié

- Bit-perfect réel et changement de fréquence sur RME ADI-2 Pro / Merging Anubis.
- Chemin natif YouTube complet (audio exclusif + synchro vidéo + SponsorBlock) sur Mac.
- Tout le fonctionnement sur Windows (compile, jamais lancé par l'utilisateur).
- Lecteurs externes (VLC/mpv/IINA) en conditions réelles.

## 6. Reste à faire (backlog proposé)

**Priorité haute**
1. Valider les vidéos YouTube et SponsorBlock sur Mac.
2. Premier test Windows (installeur NSIS).
3. Tests DAC : exclusivité, 44,1 ↔ 48 kHz, `tap_device` du pont DSP.

**Priorité moyenne**
4. Faire passer l'audio Twitch / directs par le moteur Aurex (donc par le pont DSP).
5. Rééchantillonnage de secours (crate `rubato`) quand le DAC refuse une fréquence.
6. Connexion Twitch/YouTube (OAuth), chaînes suivies, notifications natives « en direct », icône de barre des menus.
7. Chat Twitch.

**Hygiène**
8. Signature Apple Developer ID + notarisation (supprimer la manip `xattr -cr`).
9. Mise à jour automatique de yt-dlp (YouTube change souvent).
10. Licences : libmpv/ffmpeg (GPL/LGPL selon build), yt-dlp (Unlicense), Deno (MIT) ; risque CGU YouTube assumé.

## 7. Comment tester une nouvelle version (Mac)

1. Télécharger l'artifact `aurex-aarch64-apple-darwin` du dernier run *Build* sur GitHub Actions.
2. Remplacer `Aurex.app` dans `/Applications`.
3. `xattr -cr /Applications/Aurex.app && open /Applications/Aurex.app`
4. En cas de problème : Réglages → « Exporter les journaux » et transmettre `Aurex-logs.txt`.
