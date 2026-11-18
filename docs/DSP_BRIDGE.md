# Pont DSP (FFI C) — brancher votre code C++

Le moteur (`audio-engine`) expose le signal en **deux points** de la boucle temps réel du DAC :

```
ffmpeg → f32 natif → ring (250 ms) ─► [1] process()    f32 entrelacé, format SOURCE, modifiable
                                      quantification exacte (sans gain ni dither)
                                      [2] tap_device()  octets EXACTS envoyés au DAC, lecture seule ─► DAC exclusif
```

* `[1] process` : pour appliquer un traitement (égaliseur, correction de salle…). Un plugin qui définit `process` est considéré comme **modifiant le signal** → l'UI retire le badge bit-perfect (« DSP actif »).
* `[2] tap_device` : pour mesurer / vérifier. Reçoit les octets finaux (16, 24 packé ou 32 bits justifié à gauche). Un plugin « tap seul » (`process = NULL`) laisse le flux intact octet pour octet.

Contrat complet et contraintes temps réel : `include/aurex_dsp.h` (ABI v1, `struct_size` vérifié).

## Écrire un plugin

```cpp
#include "aurex_dsp.h"
static int32_t prepare(void*, const AurexDspFormat* f) { /* allouer ici, jamais en process() */ return 0; }
static void process(void*, float* buf, uint32_t frames, uint32_t ch) { /* en place */ }
static AurexDspApi api = { AUREX_DSP_ABI_VERSION, sizeof(AurexDspApi), nullptr, /*latency*/0,
                           prepare, process, nullptr, nullptr, nullptr };
extern "C" AUREX_DSP_EXPORT const AurexDspApi* aurex_dsp_entry(void) { return &api; }
```

Exemples compilables : `examples/dsp-plugin/gain_plugin.cpp` (C++), `tap_only_plugin.c` (C).

```bash
g++ -shared -fPIC -std=c++17 -Iinclude examples/dsp-plugin/gain_plugin.cpp -o gain.so     # Linux
clang++ -dynamiclib -std=c++17 -Iinclude examples/dsp-plugin/gain_plugin.cpp -o gain.dylib # macOS
cl /LD /std:c++17 /Iinclude examples\dsp-plugin\gain_plugin.cpp /Fegain.dll               # Windows
```

## Charger

Dans l'UI : « Plugin DSP » → chemin → **Charger** (commande `dsp_load_plugin`). Effet **au prochain titre** (ou changement de fréquence) : `prepare()` est rappelé à chaque (ré)ouverture du DAC, avec la fréquence réelle. Si `prepare` renvoie ≠ 0, le plugin est contourné et l'erreur remontée.

Alternative : lier votre code statiquement dans le binaire et appeler `aurex_dsp_install(&api)`.

## Garanties et limites

* Latence annoncée (`latency_frames`) compensée dans l'horloge A/V.
* Désinstallation : période de grâce de 100 ms avant `release()` pour laisser sortir le thread audio.
* **Périmètre V1** : seul l'audio **YouTube VOD** passe par le pont. Twitch et les directs YouTube sont rendus par mpv (sortie exclusive de mpv) et n'y passent pas ; migration prévue (V1.1) en décodant l'audio nous-mêmes.
* Vérifié ici : chargement de vrais plugins C et C++ (Linux, backend factice) et preuve bit-exact du chemin `tap_device`. **Non vérifié** : le même chemin sur CoreAudio/WASAPI et sur vos interfaces.
