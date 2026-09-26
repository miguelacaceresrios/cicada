//! Aleatoriedad sin estado. Cada decisión es un hash de su contexto, de modo
//! que no depende del orden en que se consulta el tiempo: reproducir desde el
//! ciclo 40, renderizar offline o recargar en vivo dan lo mismo.

fn splitmix(mut x: u64) -> u64 {
    x = x.wrapping_add(0x9E37_79B9_7F4A_7C15);
    x = (x ^ (x >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    x ^ (x >> 31)
}

pub fn hash_str(s: &str) -> u64 {
    s.bytes().fold(0xCBF2_9CE4_8422_2325, |h, b| {
        (h ^ b as u64).wrapping_mul(0x100_0000_01B3)
    })
}

pub fn hash(parts: &[u64]) -> u64 {
    parts.iter().fold(0, |h, &p| splitmix(h ^ p))
}

/// Un valor en [0, 1) a partir de un hash.
pub fn unit(h: u64) -> f64 {
    (h >> 11) as f64 / (1u64 << 53) as f64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stable_and_spread() {
        assert_eq!(hash(&[1, 2, 3]), hash(&[1, 2, 3]));
        assert_ne!(hash(&[1, 2, 3]), hash(&[1, 3, 2]));
        let mean = (0..10_000).map(|i| unit(hash(&[7, i]))).sum::<f64>() / 10_000.0;
        assert!((mean - 0.5).abs() < 0.02, "media {mean}");
    }
}
