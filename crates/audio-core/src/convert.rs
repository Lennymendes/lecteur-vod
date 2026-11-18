//! Conversion PCM f32 -> représentation du DAC.
//!
//! Règle « bit-perfect » : aucun gain, aucun dither, aucun filtre. On ne fait que
//! quantifier avec un arrondi au plus proche. Pour une source 16 bits décodée en
//! f32 (v / 32768), la requantification en 16 bits est *exacte* ; idem en 24 bits
//! (f32 a 24 bits de mantisse). Le volume doit être fait par le DAC / le matériel,
//! pas ici.

use crate::SampleKind;

/// Quantifie `x` (∈ [-1, 1[) sur `bits` bits signés, arrondi au plus proche, saturé.
#[inline]
pub fn quantize(x: f32, bits: u8) -> i64 {
    debug_assert!((2..=32).contains(&bits));
    let max = (1u64 << (bits - 1)) as f64;
    let v = (x as f64 * max).round();
    v.clamp(-max, max - 1.0) as i64 // NaN -> 0
}

#[inline]
fn pick(src: &[f32], frame: usize, ch: usize, src_ch: usize) -> f32 {
    if src_ch == 1 {
        src[frame] // mono -> dupliqué sur tous les canaux
    } else if ch < src_ch {
        src[frame * src_ch + ch]
    } else {
        0.0
    }
}

/// Écrit `src` (f32 entrelacé, `src_ch` canaux) dans `dst` (`dst_ch` canaux, format `kind`).
/// Retourne le nombre de frames écrites. Aucune allocation : utilisable dans le callback temps réel.
pub fn encode(kind: SampleKind, src: &[f32], src_ch: usize, dst_ch: usize, dst: &mut [u8]) -> usize {
    let bps = kind.bytes_per_sample();
    let frames = (src.len() / src_ch.max(1)).min(dst.len() / (dst_ch * bps));
    for f in 0..frames {
        for c in 0..dst_ch {
            let x = pick(src, f, c, src_ch);
            let o = (f * dst_ch + c) * bps;
            let out = &mut dst[o..o + bps];
            match kind {
                SampleKind::F32 => out.copy_from_slice(&x.to_le_bytes()),
                SampleKind::I16 => out.copy_from_slice(&(quantize(x, 16) as i16).to_le_bytes()),
                SampleKind::I24Packed => out.copy_from_slice(&(quantize(x, 24) as i32).to_le_bytes()[..3]),
                SampleKind::I32 { valid_bits } => {
                    let q = quantize(x, valid_bits) << (32 - valid_bits as u32);
                    out.copy_from_slice(&(q as i32).to_le_bytes());
                }
            }
        }
    }
    frames
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn s16_roundtrip_is_exact() {
        for v in (i16::MIN..=i16::MAX).step_by(7).chain([i16::MIN, -1, 0, 1, i16::MAX]) {
            let x = v as f32 / 32768.0;
            assert_eq!(quantize(x, 16), v as i64, "v={v}");
        }
    }

    #[test]
    fn s24_roundtrip_is_exact() {
        for k in (-(1i64 << 23)..(1 << 23)).step_by(4099) {
            let x = k as f32 / 8_388_608.0;
            assert_eq!(quantize(x, 24), k, "k={k}");
        }
    }

    #[test]
    fn saturates_and_nan() {
        assert_eq!(quantize(2.0, 16), 32767);
        assert_eq!(quantize(-2.0, 16), -32768);
        assert_eq!(quantize(f32::NAN, 24), 0);
    }

    #[test]
    fn i32_container_24_is_left_justified() {
        let mut out = [0u8; 4];
        encode(SampleKind::I32 { valid_bits: 24 }, &[0.5], 1, 1, &mut out);
        // 0.5 -> 0x400000 sur 24 bits -> 0x40000000 une fois justifié à gauche
        assert_eq!(i32::from_le_bytes(out), 0x4000_0000);
    }

    #[test]
    fn packed24_layout() {
        let mut out = [0u8; 3];
        encode(SampleKind::I24Packed, &[-1.0], 1, 1, &mut out);
        assert_eq!(out, [0x00, 0x00, 0x80]); // -8388608
    }

    #[test]
    fn mono_to_stereo_duplicates() {
        let mut out = [0u8; 4];
        let n = encode(SampleKind::I16, &[0.25], 1, 2, &mut out);
        assert_eq!(n, 1);
        assert_eq!(i16::from_le_bytes([out[0], out[1]]), i16::from_le_bytes([out[2], out[3]]));
    }
}
