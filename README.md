# Aurex

Lecteur desktop **Twitch + YouTube** pour audiophiles (macOS / Windows) : sortie audio **exclusive** (CoreAudio hog mode / WASAPI Exclusive), fréquence du DAC alignée sur celle du flux (44,1 ↔ 48 kHz), anti-pub Twitch (playlist HLS nettoyée en local), SponsorBlock + zéro pub sur YouTube (voie native), et un **pont DSP C/C++** juste avant le DAC.

> **État : V1.0 MVP, non compilée de bout en bout.** Le cœur (audio, HLS, YouTube natif) est testé sous Linux avec un backend factice et un vrai ffmpeg. CoreAudio, WASAPI, Tauri, libmpv, les surfaces vidéo natives et l'UI n'ont **pas** été compilés ni exécutés. Lisez `ARCHITECTURE.md` §0 avant de commencer ; la CI est faite pour la première compilation réelle.

## Démarrer
Voir [`docs/BUILD.md`](docs/BUILD.md). En bref : `npm run setup`, `scripts/fetch-sidecars.sh`, `npm run dev`.

## Documentation
* [`ARCHITECTURE.md`](ARCHITECTURE.md) — choix, flux, décisions V1, statut de vérification, feuille de route
* [`docs/DSP_BRIDGE.md`](docs/DSP_BRIDGE.md) — brancher votre code C++ avant le DAC
* [`docs/SPIKE1.md`](docs/SPIKE1.md) — UI transparente au-dessus de la vidéo native (risques et replis)
* [`docs/BUILD.md`](docs/BUILD.md) — compilation, libmpv, licences

## Limites connues de la V1.0
* Twitch et directs YouTube : audio rendu par mpv (exclusif) — **hors pont DSP**, bit-perfect non affirmé.
* Pas de rééchantillonnage de repli si le DAC refuse la fréquence du flux (erreur explicite).
* Pas encore : OAuth, notifications, chat.
* L'extraction YouTube native contrevient aux CGU de YouTube (choix assumé).
