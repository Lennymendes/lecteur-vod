# Spike 1 — UI React transparente au-dessus de la vidéo native

## Principe
Une seule fenêtre Tauri, `transparent: true`. Sous la webview, une **surface native** (`src-tauri/src/surface/`) donnée à libmpv via `wid`. La page ne dessine rien (fond transparent) dans le `VideoSlot` ; l'UI en envoie le rectangle (`video_set_rect`, pixels CSS + devicePixelRatio), le cœur Rust place la surface.

| OS | Surface | Ordre Z |
|---|---|---|
| Windows | `CreateWindowExW` (`STATIC`, `WS_CHILD`) enfant de la fenêtre principale | `SetWindowPos(HWND_BOTTOM)` : sous WebView2 |
| macOS | `NSView` ajoutée à la vue contenu | `addSubview:positioned:NSWindowBelow` : sous `WKWebView` ; `macOSPrivateApi` pour le fond transparent |

## ⚠ Ce qui n'est PAS vérifié (aucun de ces deux chemins n'a été compilé ni exécuté)
1. **Windows** : WebView2 doit laisser voir un HWND *frère* au travers de ses zones transparentes. Selon version/GPU/DirectComposition, cela peut échouer (zone noire ou vidéo masquée).
2. **macOS** : libmpv avec `wid` = `NSView*` ajoute sa propre sous-vue ; sur Apple Silicon, `vo=gpu` + `hwdec` doivent être validés ; l'ordre Z après redimensionnement aussi.
3. Les API `windows` 0.58 et `objc2` 0.2 : signatures susceptibles d'avoir bougé (erreurs de compilation possibles).

## Critères de réussite (à tester)
- Vidéo visible sous un bouton semi-transparent de l'UI ; redimensionnement fluide ; plein écran ; HiDPI 125/150 % (Windows) et Retina.
- Aucun clignotement au changement d'onglet ; clics traversant correctement le `VideoSlot`.

## Replis si ça échoue
- **A. Deux fenêtres** : fenêtre vidéo (mpv) + fenêtre UI sans bordure, transparente, au-dessus, synchronisées en position/taille (`WindowEvent::Moved/Resized`). Robuste sur Windows, plus lourd à gérer (focus, ombre).
- **B. Contrôles en fenêtre enfant** : UI complète dans la fenêtre principale ; seule la barre de contrôles superposée vit dans une petite webview enfant.
- **C. Rendu dans la page** : mpv `render API` (OpenGL) vers une texture — change l'architecture, dernier recours.
