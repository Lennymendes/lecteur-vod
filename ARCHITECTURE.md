# Aurex — lecteur Twitch + YouTube « audiophile » (macOS / Windows)

Document d'architecture + code de référence. **V1.0 (MVP)** — 18 nov. 2026. Décision YouTube : **voie native** (validée).

## 0. Lecture rapide : trois points durs à connaître avant de coder

1. **« Bit-perfect » ne veut pas dire « sans perte » ici.** Twitch/YouTube diffusent de l'AAC ou de l'Opus (lossy). Ce qu'on garantit : *aucune altération après le décodeur* — pas de mixeur OS, pas de rééchantillonnage OS, pas de gain, pas de dither — et la fréquence du DAC = celle du flux. C'est exactement ce que fait le code de `crates/audio-engine`.
2. **Le mode exclusif coupe tous les autres sons du système** (notifications, autres applis) tant que le flux joue ; il impose aussi de renoncer au volume logiciel (il casserait le bit-perfect → volume sur le DAC/ampli). Il faut un interrupteur « Exclusif » et un relâchement automatique en pause.
3. **YouTube : voie native retenue** (flux extraits par yt-dlp, décodés par ffmpeg, joués par `audio-engine`). Elle permet bit-perfect + SponsorBlock + zéro pub, et **viole les conditions d'utilisation de YouTube** — choix assumé. La voie « conforme » (lecteur embarqué) est abandonnée en V1.

**Statut de vérification (honnête).**
- ✔ Exécuté et vert (Linux, ffmpeg réel) : `audio-core` (15 tests), `audio-engine` (rate switch, pause/flush, pont DSP), `hls-playlist` (9), `yt-native` (13 unitaires + pipeline 4 + **preuve bit-exact** : WAV 16 bits → octets envoyés au DAC identiques). ABI C du pont DSP chargée avec de vrais plugins C et C++.
- ✘ **Jamais compilé ici** (pas d'accès crates.io/npm) : backends CoreAudio/WASAPI, `hls-guard`, chargeur de plugin (`libloading`), `extract.rs`/`fetch_segments` (feature `net`), shell Tauri, surfaces vidéo Win32/AppKit, wrapper libmpv, UI React. La CI GitHub (`.github/workflows/ci.yml`) est faite pour la **première compilation réelle** : attendez-vous à quelques erreurs de compilation à corriger au premier passage.
- ✘ Non testé sur matériel : RME ADI-2 Pro, Merging Anubis, exclusivité réelle, commutation 44,1/48 kHz sur DAC.

---

## 1. Choix technologique

**Recommandation : Tauri 2 (cœur Rust) + UI React/TypeScript + libmpv pour le média, `audio-engine` Rust pour la sortie exclusive contrôlée.**

| Critère | Tauri 2 + Rust | Electron + addon natif (napi-rs) | C++/Qt |
|---|---|---|---|
| Audio exclusif, threads temps réel | Natif Rust : un seul langage du réseau au DAC | Addon natif obligatoire, 2 mondes à ponter | Excellent |
| Réseau/HLS (proxy, parsing) | tokio/axum/reqwest, sûr et rapide | Node : OK | Possible, plus lourd |
| UI « haut de gamme » (React, animations) | WebView système (WKWebView / WebView2) | Chromium embarqué, rendu identique partout | QML : bon, mais écosystème UI plus petit |
| Poids / RAM | ~10 Mo, RAM faible | 150 Mo+, RAM élevée | Moyen |
| Vidéo dans l'app | Le WebView ne sert qu'à l'UI ; vidéo native (mpv) | `<video>` + MSE possible | Natif |
| Risque principal | « Airspace » : superposer l'UI web à une vue vidéo native (voir Spike 1) | Poids, et on écrit quand même le natif | UI web plus difficile |

Pourquoi pas Electron : on écrirait la partie critique (audio + réseau) en natif de toute façon ; Chromium n'apporte ici que du poids. Pourquoi pas Qt : l'UI et les connexions OAuth/chat sont plus rapides à itérer en React.

**Moteur média : deux voies, on commence par la A.**

- **Voie A (v1, recommandée) — libmpv.** mpv fait démux HLS, décodage, rendu vidéo et synchro A/V ; on lui demande la sortie exclusive : `--ao=wasapi --audio-exclusive=yes` sous Windows, `--ao=coreaudio_exclusive` sous macOS (accès direct au périphérique, sans passer par le serveur audio). mpv négocie le format avec le DAC et, en principe, ré-initialise la sortie quand le format audio change (à confirmer au Spike 2). Rapide à livrer, éprouvé. Limites : politique de repli peu visible (on ne sait pas toujours si l'on est retombé en partagé → on lit `audio-out-params/samplerate` et on affiche un badge), bugs connus de `coreaudio_exclusive` selon certains DAC 24 bits.
- **Voie B (phase 3, si A ne suffit pas) — pipeline maison.** `ffmpeg-next` (démux HLS + décodage) → `AudioEngine::push(StreamFormat, pcm)` → DAC ; vidéo rendue par wgpu/vue native, calée sur `AudioEngine::presented_secs()` (horloge maître = audio). Contrôle total : détection de fréquence avant ouverture, liste des formats du DAC, volume matériel, état « exclusif / repli » exact. C'est le code livré dans `audio-engine` ; il est prêt pour B mais pas encore branché à un décodeur.

---

## 2. Architecture d'ensemble

```
┌──────────────────────────── UI (React, WebView) ────────────────────────────┐
│ Onglets Twitch | YouTube (tous deux montés, un seul visible)  · Chat · Notifs│
└───────────────▲──────────────────────────────────────────────────┬──────────┘
        événements Tauri                                  commandes Tauri (IPC)
┌───────────────┴──────────────────────────────────────────────────▼──────────┐
│                           Cœur Rust (src-tauri)                              │
│  auth/        Twitch Device Code · Google OAuth+PKCE (loopback) · keychain  │
│  twitch/      Helix · EventSub WS (stream.online) · chat                    │
│  youtube/     Data API v3 · SponsorBlock · YouTubeSource (embed | native)   │
│  notify/      notifications natives + icône tray / barre des menus          │
│  player/      libmpv (voie A)  |  decode+sync+audio-engine (voie B)         │
│                                                                              │
│  hls-guard ──► proxy HTTP 127.0.0.1:port/s/<secret>/<chaîne>.m3u8 ──► mpv   │
│     │  GQL token → usher → playlist média → détection pub → backup/filtre   │
│  audio-engine ─► CoreAudio hog mode / WASAPI exclusive ─► DAC               │
└──────────────────────────────────────────────────────────────────────────────┘
```

Principes :
- **Le lecteur ne parle jamais à Twitch directement** : il lit une playlist locale déjà nettoyée. L'anti-pub est ainsi indépendant du lecteur (mpv aujourd'hui, pipeline maison demain).
- **Interrupteurs à la volée** : `Guard::set_enabled(bool)` (Twitch) change la politique sans relancer la lecture (la numérotation reste stable) ; SponsorBlock = simple drapeau lu par la boucle de saut.
- **Onglets « instantanés »** : l'UI des deux onglets reste montée (`display:none`, pas de démontage) ; la session `Guard` du dernier flux reste chaude (jeton + playlist maître déjà résolus), donc le retour sur un flux relance en < 1 s. Deux sorties exclusives ne peuvent pas coexister : un seul flux audio actif à la fois.

---

## 3. Contournement HLS Twitch (état 2026)

**Le problème.** Depuis « SureStream », les pubs Twitch sont *insérées côté serveur dans le même flux HLS* que la diffusion : un bloqueur réseau classique qui coupe la requête pub coupe aussi le flux, d'où l'écran violet « pause publicitaire ». La seule approche qui fonctionne en 2026 consiste à travailler **au niveau de la playlist M3U8** : détecter le marqueur de pub, puis soit récupérer une playlist propre, soit retirer les segments pub.

**Étape 1 — obtenir le M3U8 sans le lecteur web** (`hls-guard/src/twitch.rs`) :
1. `POST https://gql.twitch.tv/gql`, opération `PlaybackAccessToken` avec le `Client-ID` public du site web, `playerType` choisi → `(value, signature)`. Requête GraphQL en **texte brut** : les hash « persisted query » tournent sans préavis.
2. `GET https://usher.ttvnw.net/api/channel/hls/<chaîne>.m3u8?sig=…&token=…&allow_source=true&allow_audio_only=true&player_backend=mediaplayer&platform=web&supported_codecs=avc1&p=<aléa>` → playlist maître.
3. Choisir une variante (`chunked` = source) → playlist média live (rafraîchie toutes les ~2 s).

**Étape 2 — détecter la pub** (`hls-playlist`). Signaux observés dans les outils communautaires : `#EXT-X-DATERANGE` de classe `twitch-stitched-ad` (ou `twitch-maf-ad`, ID `stitched-ad-*`, attributs `X-TV-TWITCH-AD-*`), `#EXT-X-CUE-OUT/IN`, ancien titre `EXTINF` « Amazon ». Un segment est une pub si son `PROGRAM-DATE-TIME` tombe dans la plage du DATERANGE. Piège : le DATERANGE `twitch-trigger` est présent en permanence, **ce n'est pas une pub** (testé).

**Étape 3 — politique** (`hls-guard/src/session.rs`), par préférence :
1. Playlist propre → passage tel quel.
2. Pub détectée → **flux de secours** : redemander jeton + usher avec un autre `playerType` (`embed`, `autoplay`, `picture-by-picture`… valeurs vues dans des outils communautaires, à ajuster par télémétrie) et prendre la première playlist sans marqueur. Retour périodique (20 s) au flux principal.
3. Aucun flux propre → **retrait des segments pub** (`LivePlaylistFilter`) : numérotation de sortie monotone, `#EXT-X-DISCONTINUITY` après le trou, identité des segments par `PROGRAM-DATE-TIME` (donc compatible avec un changement de session de secours). Le lecteur attend la fin de la coupure ; l'UI reçoit `GuardEvent::Stripped` pour afficher « coupure pub ≈ N s » au lieu d'un écran figé.

**Étape 4 — servir à mpv** (`proxy.rs`) : mini-serveur axum sur `127.0.0.1`, port éphémère, secret aléatoire dans l'URL et contrôle du `Host` (anti DNS-rebinding). Seule la playlist transite ; les `.ts` sont téléchargés directement au CDN par mpv.

**Réalités à accepter.**
- C'est du chat-et-souris : Twitch modifie marqueurs, jetons et en-têtes. Garder `hls-playlist` isolé et couvert par des fixtures ; prévoir une **mise à jour de règles à distance** (JSON signé) pour ne pas republier l'app à chaque changement.
- L'API GQL/usher est **non documentée**. Un `Client-Integrity` peut devenir obligatoire ; non vérifié ici (pas d'accès réseau depuis le bac à sable de rédaction).
- Aucun contournement n'est garanti à 100 % : le repli 3 laisse une coupure.
- Voie légitime : avec Turbo ou un abonnement à la chaîne, Twitch ne sert pas de pubs, mais c'est le jeton *du site web* (cookie `auth-token`) qui compte, pas le jeton Helix OAuth officiel de l'app (`TwitchHls::new(web_oauth)`).
- Hors périmètre : les pubs de VOD sont insérées côté client (autre mécanisme, requêtes `edge.ads.twitch.tv`).
- Conformité : cela contrevient aux conditions d'utilisation de Twitch ; risque de blocage du compte/de l'IP. À votre appréciation pour un usage personnel ; ne pas distribuer largement sous un Client-ID qui vous appartient.

---

## 4. YouTube, SponsorBlock, anti-pub

**API officielle (conforme)** — OAuth Google, flux *code d'autorisation + PKCE* avec redirection loopback `127.0.0.1` : `subscriptions.list(mine=true)`, `videos.rate` (likes), `liveChatMessages.list/insert` (chat), détection des directs des abonnements. Attention au quota (10 000 unités/jour par défaut — à vérifier dans la console Google Cloud) : le chat live en polling est coûteux.

**Lecture : trait `YouTubeSource`, deux implémentations.**

| | `Embed` (IFrame API officielle) | `Native` (flux extraits → mpv/pipeline maison) |
|---|---|---|
| Pubs | Présentes (sauf session Premium, mais Google bloque la connexion dans un WebView embarqué) | Absentes du flux |
| Bit-perfect | **Impossible** (audio dans le WebView → mixeur OS) | Oui |
| SponsorBlock | `seekTo()` côté JS | Saut natif exact |
| Conformité | Conforme **sauf** blocage de pubs/altération du lecteur | **Viole les CGU YouTube**, maintenance continue (extracteur) |
| Risque technique | Erreur de lecture sans `Referer` valide (les clients IFrame doivent s'identifier par l'en-tête Referer) — servir l'UI depuis une origine HTTP(S) locale propre | Cassures fréquentes de l'extracteur |

Points de politique à connaître : les règles de l'API YouTube interdisent de « bloquer, modifier ou remplacer les publicités » jouées via le service API et de modifier la lecture standard du lecteur ; YouTube a annoncé durcir l'application de ces règles contre les applis tierces qui bloquent les pubs (erreurs du type « contenu indisponible dans cette appli »). Un AdBlock réseau dans un WebView qui charge le lecteur embarqué relève exactement de cela.

**Ma recommandation d'architecture** : implémenter `Native` derrière un drapeau de build/réglage, garder `Embed` comme voie par défaut et de secours, et **ne jamais mélanger** la clé d'API officielle (comptes, abonnements, likes, chat) avec la voie `Native` : deux clients distincts, pour qu'un blocage de l'une n'emporte pas l'autre.

**SponsorBlock (voie `Native`, sans « injection de script »).** Requête à préservation de vie privée : `GET https://sponsor.ajay.app/api/skipSegments/<4 premiers hex de sha256(videoID)>?categories=["sponsor","selfpromo","interaction","intro","outro"]&actionTypes=["skip"]`, puis filtrer la réponse sur le `videoID` exact. Boucle : observer `time-pos` de mpv ; si `start ≤ t < end` → `seek end absolute`, avec toast « Sponsor passé — Annuler ». Dans `Embed`, même logique via `player.getCurrentTime()/seekTo()` (zone grise vis-à-vis des CGU).

---

## 5. Audio bit-perfect (code : `crates/audio-engine`, `crates/audio-core`)

**Chaîne** : décodeur → `AudioEngine::push(StreamFormat{rate,channels}, pcm_f32)` → ring SPSC (250 ms) → callback temps réel du backend → `convert::encode` (arrondi au plus proche, **sans dither ni gain**) → DAC en exclusif.

**Détection de fréquence et bascule automatique.** La fréquence vient du **décodeur** (paramètres du codec), pas d'un reniflage des segments : l'AAC peut déclarer une fréquence différente de la sortie (HE-AAC/SBR), Opus sort toujours à 48 kHz, et une rupture HLS (coupure pub, changement de source) peut changer de fréquence en cours de route. `push()` compare le format à celui du DAC ; s'il diffère : laisser finir le ring + latence → `close()` (restaure l'état du DAC) → rouvrir à la nouvelle fréquence → horloge cumulée (`presented_secs()` reste monotone, testé 44,1 → 48 kHz).

**macOS (`backend/coreaudio.rs`)** — HAL CoreAudio direct :
1. périphérique par défaut → premier stream de sortie ;
2. *physical format* entier entrelacé à la fréquence **exacte** (préférence 24/32 bits > 16 bits > float) ;
3. `kAudioDevicePropertyHogMode` = notre PID (le mixeur n'est plus dans le chemin), vérifié par relecture ; échec → `DeviceBusy` ;
4. application du format puis **attente de la prise d'effet** (asynchrone) + alignement de `NominalSampleRate` ;
5. IOProc temps réel, zéro allocation, silence (jamais de bruit) en cas d'underrun ;
6. à la fermeture : arrêt, restauration du format d'origine, libération du hog mode.

**Windows (`backend/wasapi_excl.rs`)** — WASAPI exclusif piloté par événement, dans un thread dédié (COM non `Send`). Négociation par *tentative d'ouverture* : 24-en-32, 32, 24 packé, 16 bits × plusieurs périodes de tampon (certains pilotes refusent la période par défaut : `BUFFER_SIZE_NOT_ALIGNED`). `Initialize` réussi = le DAC accepte ce format tel quel. Tampon prérempli (silence) avant `Start`. Codes d'erreur mappés vers `DeviceBusy` / `RateUnsupported`.

**Si le DAC refuse la fréquence** : `AudioError::RateUnsupported{ suggested }` propose les multiples entiers (44,1 → 88,2/176,4). L'appelant peut sur-échantillonner lui-même (rubato) puis repousser à ce débit ; **jamais de repli silencieux vers le mode partagé** (le badge UI indique « Exclusif 24 bits / 44,1 kHz » ou « Repli partagé »).

**Réglages impératifs côté décodeur/mpv** : pas de downmix, DRC, normalisation, ReplayGain ni filtre `af`. Volume : désactiver le volume logiciel en mode bit-perfect (volume du DAC/ampli), ou proposer un volume logiciel *explicitement* marqué « hors bit-perfect ».

**Voie A — options mpv équivalentes** (libmpv) :
```
ao=wasapi  audio-exclusive=yes        # Windows  (--wasapi-exclusive-buffer=default|min pour la latence)
ao=coreaudio_exclusive                # macOS   (accès direct, hog mode)
replaygain=no   af=   audio-normalize-downmix=no   volume=100
profile=low-latency                   # direct Twitch
# affichage du badge : observer audio-params/samplerate et audio-out-params/samplerate
```

**À faire avant de dire « c'est fini »** : écoute et mesure sur un vrai DAC (boucle numérique S/PDIF ou analyseur) ; listener de débranchement (`DeviceIsAlive` côté macOS, `shared.device_lost` déjà prévu) ; priorité MMCSS « Pro Audio » du thread WASAPI ; choix explicite du périphérique ; optimisation « ne pas relâcher le hog mode entre deux ouvertures ».

---

## 6. Structure du projet (V1.0)

```
aurex/
├─ Cargo.toml                      # workspace : crates/* + src-tauri
├─ include/aurex_dsp.h             # ABI C du pont DSP (v1)
├─ examples/dsp-plugin/            # plugin C++ (gain) et C (tap seul)
├─ crates/
│  ├─ audio-core/    ✔ testé       formats, conversion, ring SPSC, horloge (Shared), DspBridge, RenderCtx
│  ├─ audio-engine/  ✔ moteur+null ✘ CoreAudio/WASAPI  AudioEngine, backends, chargeur de plugin
│  ├─ hls-playlist/  ✔ testé       parsing, détection pub, filtre live
│  ├─ hls-guard/     ✘ non compilé GQL/usher, politique anti-pub, proxy HTTP local
│  └─ yt-native/     ✔ cœur testé  ✘ extract/SponsorBlock réseau   format, decode (ffmpeg), playback, servo A/V
├─ src-tauri/        ✘ non compilé Tauri 2 : commands, state, surface/{win,mac,stub}, video/{mpv,null}
├─ ui/               ✘ non buildé  React+Vite+TS : onglets persistants, VideoSlot, dock, badge
├─ scripts/          fetch-sidecars.{sh,ps1}
├─ docs/             DSP_BRIDGE.md, SPIKE1.md, BUILD.md
└─ .github/workflows/ ci.yml (tests + check), release.yml (bundle complet, manuel)
```

**Pas encore implémenté (hors V1.0)** : OAuth Twitch/Google, notifications « streamer en direct », chat, rééchantillonnage (rubato) quand le DAC refuse la fréquence du flux, Opus→DAC 44,1 kHz. Les emplacements décrits dans la version précédente de ce document (`auth/`, `twitch/helix.rs`, `notify.rs`) restent valables.

---

## 7. Décisions d'implémentation V1.0

**Deux chemins audio, un seul propriétaire du DAC.**
| Source | Vidéo | Audio | Pont DSP | Bit-perfect affirmé |
|---|---|---|---|---|
| YouTube (VOD) | libmpv, flux vidéo seul, `aid=no` | ffmpeg → f32 natif → `audio-engine` (exclusif) | **oui** | oui (si aucun DSP actif) |
| YouTube direct, Twitch | libmpv | libmpv, sortie exclusive de mpv (`wasapi`+`audio-exclusive` / `coreaudio_exclusive`) | **non** | non affirmé (mpv décide) |

Changer de source arrête d'abord le propriétaire courant (`AppState::stop_all` : drop de `Playback` → `AudioEngine::close()` libère le DAC), puis démarre le suivant. Jamais deux propriétaires.

**Fréquence dynamique.** La fréquence/les canaux sont lus dans la sortie de sondage de ffmpeg (pas de `-ar`/`-ac`). `AudioEngine::begin_track` rouvre le DAC si le format diffère du titre précédent (44,1 ↔ 48 kHz). `prepare()` du plugin DSP est rappelé à chaque réouverture.

**Synchro A/V.** L'horloge audio (frames réellement consommées par le DAC, moins la latence) est maître. mpv lit la vidéo seule en `video-sync=desync` ; un servo ajuste `speed` (±5 %, bande morte 12 ms) et recale par seek au-delà de 150 ms. La vidéo démarre quand l'audio sort.

**SponsorBlock** : segments par préfixe de hash, sauts exacts sur l'horloge audio, annulation et reset sur seek utilisateur ; interrupteur à la volée. **Anti-pub Twitch** : `Guard` + proxy local ; interrupteur à la volée (`set_enabled`).

**Pont DSP** : voir `docs/DSP_BRIDGE.md`. **Spike 1 (surface vidéo)** : voir `docs/SPIKE1.md`.

**Navigation.** Deux panneaux (Twitch, YouTube) toujours montés ; bascule = `hidden`, donc instantanée ; la session Twitch reste chaude. Le lecteur est global (dock du bas) : on peut parcourir YouTube pendant qu'un direct Twitch joue. Le `VideoSlot` n'est visible que dans l'onglet de la source en lecture.

**Sidecars** : yt-dlp et ffmpeg (externalBin). yt-dlp change souvent : mettez-le à jour en remplaçant le binaire. Licences : prenez un ffmpeg LGPL (sans `--enable-gpl`) et respectez les obligations de redistribution ; libmpv est LGPL si construit sans options GPL (à vérifier selon le build choisi).

---

## 8. Feuille de route et « spikes » à valider tôt

1. **Compilation CI** : faire passer `ci.yml` au vert (corrections attendues sur les parties jamais compilées).
2. **Spike 1 — Airspace** : surface native sous webview transparente, macOS **et** Windows (`docs/SPIKE1.md`). Repli prévu si bloquant.
3. **Spike 2 — DAC réel** : RME ADI-2 Pro / Merging Anubis ; commutation 44,1 ↔ 48 kHz en lecture ; mesure de latence ; `tap_device` pour vérifier les octets.
4. **Spike 3 — Twitch** : rejouer GQL → usher → playlist, vraies playlists pub en fixtures, `Client-Integrity` si nécessaire.
5. **V1.1** : OAuth, notifications, chat ; rééchantillonnage de repli ; Twitch audio dans le pont DSP (décodage maison au lieu de mpv).

## Sources
- Pubs Twitch en SSAI, marqueurs et backup : https://github.com/NobbyBo11ocks/no-twitch-ads · https://github.com/Spaced1127/StreamGuard · https://github.com/zurarah/twitch-ad-blocker-with-vods
- GQL / usher : https://docs.rs/crate/twitch-hls-client/1.3.8/source/src/constants.rs · https://apify.com/abotapi/twitch-scraper.md · https://streamlink.github.io/cli/plugins/twitch.html
- mpv (sorties exclusives) : https://www.mintlify.com/mpv-player/mpv/av/audio-output · https://github.com/mpv-player/mpv/issues/4478
- WASAPI exclusif : https://learn.microsoft.com/en-ie/Windows/Win32/coreaudio/exclusive-mode-streams
- Twitch OAuth/EventSub : https://dev.twitch.tv/docs/eventsub/manage-subscriptions/ · https://www.activepieces.com/blog/twitch-api-access-tokens-and-rate-limits-2026
- SponsorBlock (requête par préfixe de hash) : https://priveetee.github.io/Docs-PipePipe/extractor/sponsorblock.html · https://wiki.sponsor.ajay.app
- Règles de l'API YouTube : https://developers.google.com/youtube/terms/developer-policies-guide · https://developers.google.com/youtube/terms/required-minimum-functionality · https://www.theregister.com/2024/04/16/youtube_ad_blocking/
