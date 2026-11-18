# Compiler et lancer Aurex

## Prérequis
Rust stable (≥ 1.86), Node 20+, et :
* **macOS** : Xcode CLT, `brew install mpv` (fournit libmpv).
* **Windows** : MSVC Build Tools, WebView2 (inclus Win11). libmpv : `libmpv-2.dll` + `mpv.lib` (voir ci-dessous).

## 1. Premier passage sans libmpv (le plus rapide)
```bash
npm run setup                       # installe l'UI
scripts/fetch-sidecars.sh           # (Windows : pwsh scripts/fetch-sidecars.ps1)
cargo test -p audio-core -p audio-engine -p hls-playlist -p yt-native   # nécessite ffmpeg dans le PATH
cargo check -p aurex --no-default-features
npx tauri dev -- --no-default-features   # lance l'app sans vidéo (audio YouTube natif OK)
```
> Les options après `--` sont transmises à cargo. Sans libmpv : pas de vidéo, pas de Twitch.

## 2. Application complète
```bash
npm run dev          # = tauri dev, feature libmpv activée par défaut
npm run build        # bundle (.app/.dmg, .msi/.exe)
```
Le crate `libmpv2` (v4) s'attend à trouver libmpv à l'édition de liens.
* macOS : `export LIBRARY_PATH="$(brew --prefix mpv)/lib"` si le linker ne la trouve pas.
* Windows : récupérez un build `mpv-dev` (ex. shinchiro / zhongfly), placez `libmpv-2.dll` à côté de l'exécutable (et déclarez-la dans `bundle.resources`), générez `mpv.lib` : `lib /def:mpv.def /out:mpv.lib /machine:x64`, puis `set LIBMPV_LIB_DIR=...` selon la doc de `libmpv2`.

## 3. Ce que la CI vérifie
`ci.yml` : tests des crates cœur sur macOS/Windows/Linux (compile CoreAudio et WASAPI), compilation des plugins DSP d'exemple, `cargo check` du shell sans libmpv. `release.yml` (manuel) : bundle complet.

## 4. Licences / distribution
yt-dlp (Unlicense) et ffmpeg : bundlez un build **LGPL** et fournissez les sources/avis requis. libmpv : LGPL ou GPL selon le build. L'extraction YouTube native contrevient aux CGU de YouTube : ne distribuez pas l'app publiquement sans en mesurer les conséquences.
