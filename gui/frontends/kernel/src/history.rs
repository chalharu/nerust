//! Shared history visibility: threshold match, this level and above.
//! Tao filters and the gtk history dialog share this single rule.

/// Threshold match: visible when `severity` reaches `minimum`.
/// Info (severity 0) shows all; errors (2) show errors only.
pub fn level_visible(minimum: u8, severity: u8) -> bool {
    severity >= minimum
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn threshold_admits_that_level_and_above() {
        assert!(level_visible(0, 2));
        assert!(level_visible(1, 1));
        assert!(level_visible(1, 2));
        assert!(!level_visible(1, 0));
        assert!(level_visible(2, 2));
        assert!(!level_visible(2, 0));
    }
}
