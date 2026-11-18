/*
 * aurex_dsp.h — ABI C stable du pont DSP d'Aurex (version 1).
 *
 * Un plugin est une bibliothèque dynamique (.dylib / .dll / .so) qui exporte :
 *
 *     AUREX_DSP_EXPORT const AurexDspApi* aurex_dsp_entry(void);
 *
 * L'hôte copie la table retournée. Chargement côté Aurex : commande `dsp_load_plugin(path)`.
 *
 * ─── Où sont appelés les callbacks ? ───
 *   prepare()     hors temps réel, à chaque (ré)ouverture du DAC (donc à chaque changement de fréquence).
 *   process()     THREAD TEMPS RÉEL du DAC. Buffer f32 entrelacé, format SOURCE (avant quantification),
 *                 modifiable sur place. Blocs ≤ max_frames (annoncé dans prepare).
 *   tap_device()  THREAD TEMPS RÉEL. Octets EXACTS envoyés au DAC (après quantification), lecture seule.
 *   reset()       THREAD TEMPS RÉEL, après un seek : vider les états internes (réverb, filtres…).
 *   release()     à la désinstallation, hors temps réel.
 *
 * ─── Contraintes temps réel (process / tap_device / reset) ───
 *   Aucune allocation, aucun verrou, aucune E/S, aucun appel bloquant, aucune exception qui sorte.
 *   Durée ≪ période du buffer du DAC (typiquement 1–10 ms). Pas de globales non initialisées en prepare().
 *
 * ─── Bit-perfect ───
 *   Un plugin dont process() est non nul est considéré comme modifiant le signal : l'UI affiche
 *   « DSP actif » et retire le badge bit-perfect. Un plugin tap-only (process = NULL) laisse le flux
 *   intact octet pour octet : c'est le mode à utiliser pour mesurer / vérifier.
 */
#ifndef AUREX_DSP_H
#define AUREX_DSP_H

#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

#if defined(_WIN32)
#  define AUREX_DSP_EXPORT __declspec(dllexport)
#else
#  define AUREX_DSP_EXPORT __attribute__((visibility("default")))
#endif

#define AUREX_DSP_ABI_VERSION 1u

typedef struct AurexDspFormat {
    uint32_t sample_rate;
    uint32_t channels;
    uint32_t max_frames;     /* plus grand `frames` que process() recevra */
} AurexDspFormat;

typedef struct AurexDeviceFormat {
    uint32_t sample_rate;
    uint32_t channels;
    uint32_t sample_kind;    /* 0 = f32, 1 = i16, 2 = i24 packé (3 octets), 3 = entier 32 bits justifié à gauche */
    uint32_t valid_bits;     /* 16, 24 ou 32 */
    uint32_t bytes_per_sample;
} AurexDeviceFormat;

typedef struct AurexDspApi {
    uint32_t abi_version;    /* = AUREX_DSP_ABI_VERSION */
    uint32_t struct_size;    /* = sizeof(AurexDspApi) */
    void*    user;           /* passé tel quel à tous les callbacks */
    uint32_t latency_frames; /* latence ajoutée par le plugin (compensée dans l'horloge A/V) */

    int32_t (*prepare)(void* user, const AurexDspFormat* fmt);                       /* 0 = OK ; sinon bypass + erreur UI */
    void    (*process)(void* user, float* interleaved, uint32_t frames, uint32_t channels);
    void    (*tap_device)(void* user, const uint8_t* data, uint32_t bytes, const AurexDeviceFormat* fmt);
    void    (*reset)(void* user);
    void    (*release)(void* user);
} AurexDspApi;

/* Pour un plugin lié STATIQUEMENT dans le binaire hôte (exports fournis par la crate audio-engine) : */
int32_t aurex_dsp_install(const AurexDspApi* api);   /* 0 = OK, -2 ABI, -3 taille, -4 aucun callback */
void    aurex_dsp_uninstall(void);
void    aurex_dsp_set_bypass(int32_t on);

#ifdef __cplusplus
}
#endif
#endif /* AUREX_DSP_H */
