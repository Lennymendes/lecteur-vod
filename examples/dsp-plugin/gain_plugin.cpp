// Plugin DSP d'exemple (C++17) : gain fixe + vumètre crête. Sert de gabarit pour votre propre code.
//   macOS  : clang++ -std=c++17 -O2 -shared -fPIC -I../../include gain_plugin.cpp -o libaurex_gain.dylib
//   Windows: cl /LD /O2 /std:c++17 /I..\..\include gain_plugin.cpp /Fe:aurex_gain.dll
//   Linux  : g++ -std=c++17 -O2 -shared -fPIC -I../../include gain_plugin.cpp -o libaurex_gain.so
#include "aurex_dsp.h"
#include <atomic>
#include <cmath>
#include <new>

namespace {
struct State {
    float gain = 1.0f;                 // 1.0 = transparent (mais process() reste « modifiant » pour l'UI)
    std::atomic<float> peak{0.0f};     // lisible depuis un autre thread
    uint32_t rate = 0, channels = 0;
};
State g_state;                          // statique : aucune allocation dans le thread temps réel

int32_t prepare(void* u, const AurexDspFormat* f) {
    auto* s = static_cast<State*>(u);
    s->rate = f->sample_rate;
    s->channels = f->channels;
    s->peak.store(0.0f);
    return 0;                           // l'allocation éventuelle de buffers se fait ICI, pas dans process()
}

void process(void* u, float* x, uint32_t frames, uint32_t channels) {
    auto* s = static_cast<State*>(u);
    const uint32_t n = frames * channels;
    float pk = s->peak.load(std::memory_order_relaxed);
    for (uint32_t i = 0; i < n; ++i) {
        x[i] *= s->gain;
        pk = std::fmax(pk, std::fabs(x[i]));
    }
    s->peak.store(pk, std::memory_order_relaxed);
}

void reset(void* u) { static_cast<State*>(u)->peak.store(0.0f); }

const AurexDspApi kApi = {
    AUREX_DSP_ABI_VERSION, sizeof(AurexDspApi), &g_state, /*latency_frames=*/0,
    prepare, process, /*tap_device=*/nullptr, reset, /*release=*/nullptr,
};
}  // namespace

extern "C" AUREX_DSP_EXPORT const AurexDspApi* aurex_dsp_entry(void) { return &kApi; }
