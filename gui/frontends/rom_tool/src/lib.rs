//! ROM tooling frontend: CLI presentation over the harness engine.
//!
//! The engine (`nerust_rom_test`) owns driving, data, and the
//! [`nerust_rom_test::report::ReportRenderer`] boundary. This crate owns
//! everything presentation: the `rom_tool` / `perf` binaries, the HTML
//! renderer, and console formatting helpers.

use std::path::{Path, PathBuf};

pub mod html;
pub mod perf;

/// Default report output root, relative to this crate's manifest dir.
pub fn default_output_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../target/rom-tests")
}

/// Compact hex preview for serial bytes: full text when short,
/// truncated with a length otherwise. Console formatting shared by
/// the CLI entry points and the HTML renderer (stdout and report
/// stay visually consistent).
pub fn hex_preview(bytes: &[u8]) -> String {
    const PREVIEW: usize = 64;
    let shown = &bytes[..bytes.len().min(PREVIEW)];
    let mut text = String::with_capacity(shown.len() * 2);
    for byte in shown {
        text.push_str(&format!("{byte:02X}"));
    }
    if bytes.len() > PREVIEW {
        text.push_str(&format!("…({} bytes)", bytes.len()));
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_preview_covers_empty_short_and_long() {
        assert_eq!(hex_preview(&[]), "");
        assert_eq!(hex_preview(b"Pass"), "50617373");
        let exact = vec![0xAB; 64];
        assert_eq!(hex_preview(&exact).len(), 128);
        assert!(!hex_preview(&exact).contains("bytes)"));
        let long = vec![0xAB; 65];
        let preview = hex_preview(&long);
        assert!(preview.starts_with(&"AB".repeat(64)));
        assert!(preview.ends_with("…(65 bytes)"));
    }

    #[test]
    fn default_output_root_points_at_target_rom_tests() {
        assert!(default_output_root().ends_with("target/rom-tests"));
    }
}
