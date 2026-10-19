//! Display-density cache keys. Densities quantize to 0.01 so float
//! jitter never re-queries platform insets every frame.

/// Quantized density key: `round(scale * 100)`, clamped to sane range.
/// Key equality is the epsilon; `u32::MAX` stays a force-miss sentinel.
pub(crate) fn density_key(scale: f32) -> u32 {
    (scale.clamp(0.25, 8.0) * 100.0).round() as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quantizes_to_hundredths() {
        assert_eq!(density_key(1.0), 100);
        assert_eq!(density_key(2.0), 200);
        assert_eq!(density_key(2.755), 276);
    }

    #[test]
    fn ignores_sub_hundredth_jitter() {
        assert_eq!(density_key(1.0), density_key(1.004));
        assert_ne!(density_key(1.0), density_key(1.006));
    }

    #[test]
    fn clamps_and_stays_deterministic() {
        assert_eq!(density_key(0.0), density_key(0.25));
        assert_eq!(density_key(100.0), density_key(8.0));
        assert_eq!(density_key(f32::NAN), density_key(f32::NAN));
        assert_ne!(density_key(1.0), u32::MAX);
    }
}
