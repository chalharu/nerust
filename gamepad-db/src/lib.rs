//! Shared gamepad math helpers (deadzone rescale).

/// Deadzone with rescale: inside the zone is `0.0`, the rim stays `1.0`.
/// `deadzone >= 1.0` parks everything at neutral instead of dividing by zero.
pub fn apply_deadzone(value: f32, deadzone: f32) -> f32 {
    if deadzone >= 1.0 || value.abs() < deadzone {
        return 0.0;
    }
    (value - deadzone.copysign(value)) / (1.0 - deadzone)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deadzone_parks_and_rescales() {
        assert_eq!(apply_deadzone(0.05, 0.1), 0.0);
        assert_eq!(apply_deadzone(-0.05, 0.1), 0.0);
        assert_eq!(apply_deadzone(1.0, 0.2), 1.0);
        assert_eq!(apply_deadzone(-1.0, 0.2), -1.0);
        assert!((apply_deadzone(0.6, 0.2) - 0.5).abs() < 1e-6);
        assert_eq!(apply_deadzone(0.9, 1.0), 0.0);
        assert_eq!(apply_deadzone(0.9, 2.0), 0.0);
    }
}
