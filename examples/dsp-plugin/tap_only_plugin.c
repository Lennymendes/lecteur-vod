/* Plugin tap-only (C99) : ne touche PAS au signal (process = NULL), compte les octets envoyés au DAC.
 * Idéal pour vérifier le bit-perfect (calculez un hash des octets, comparez à la référence).
 *   cc -O2 -shared -fPIC -I../../include tap_only_plugin.c -o libaurex_tap.so   (.dylib / .dll selon l'OS) */
#include "aurex_dsp.h"

static volatile uint64_t g_bytes = 0;
static volatile uint32_t g_rate = 0, g_bits = 0;

static int32_t prepare(void* u, const AurexDspFormat* f) { (void)u; g_rate = f->sample_rate; return 0; }
static void tap(void* u, const uint8_t* d, uint32_t n, const AurexDeviceFormat* f) {
    (void)u; (void)d; g_bytes += n; g_bits = f->valid_bits;   /* ici : hash incrémental, vumètre, FFT… */
}

static const AurexDspApi kApi = {
    AUREX_DSP_ABI_VERSION, sizeof(AurexDspApi), 0, 0, prepare, 0, tap, 0, 0,
};
AUREX_DSP_EXPORT const AurexDspApi* aurex_dsp_entry(void) { return &kApi; }
