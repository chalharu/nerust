use std::{
    path::{Path, PathBuf},
    time::SystemTime,
};

use super::{error::RomTestError, results::CaseOutcome};

pub mod html;

/// One reportable run: title, outcome counts, start time, and the
/// outcomes themselves. Data only — renderers decide presentation.
pub struct Report<'a> {
    pub title: &'a str,
    pub summary: &'a ReportSummary,
    pub started_at: SystemTime,
    pub outcomes: &'a [CaseOutcome],
}

/// Outcome counts for one ROM run. Pure data: no rendering, no I/O.
/// Renderers take this plus the outcomes and decide the presentation.
#[derive(Debug, Clone)]
pub struct ReportSummary {
    pub passed: usize,
    pub failed: usize,
    pub ignored: usize,
    pub expected_failed: usize,
}

/// Count outcomes into a [`ReportSummary`]. Total order is fixed:
/// passed, then ignored (skipped), then expected-fail; the remainder
/// is failed.
pub fn summarize(outcomes: &[CaseOutcome]) -> ReportSummary {
    let passed = outcomes.iter().filter(|outcome| outcome.passed()).count();
    let ignored = outcomes
        .iter()
        .filter(|outcome| outcome.is_skipped())
        .count();
    let expected_failed = outcomes
        .iter()
        .filter(|outcome| outcome.is_expected_failure())
        .count();
    let failed = outcomes
        .len()
        .saturating_sub(passed)
        .saturating_sub(ignored)
        .saturating_sub(expected_failed);
    ReportSummary {
        passed,
        failed,
        ignored,
        expected_failed,
    }
}

/// Presentation boundary. Data generation ([`summarize`]) stays in this
/// module; each output format implements this trait without touching
/// the counting logic.
pub trait ReportRenderer {
    /// Render `report` under `output_dir`, returning the primary
    /// artifact path (entry point a human opens first).
    fn render(&self, report: &Report<'_>, output_dir: &Path) -> Result<PathBuf, RomTestError>;
}

/// Upstream suite name derived from the manifest ROM path
/// (`collection/suite/file`): the second segment when present,
/// otherwise the first. Heuristic, documented as such: the manifest
/// carries no suite field, and adding one would churn 1300+ cases.
/// `nes-test-roms/blargg_apu_2005.07.30/01.len_ctr.nes` → `blargg_apu_2005.07.30`.
pub fn suite_name(rom: &str) -> &str {
    let mut segments = rom.split('/');
    let first = segments.next().unwrap_or(rom);
    segments.next().unwrap_or(first)
}

pub fn default_output_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../target/rom-tests")
}

/// Compact hex preview for serial bytes: full text when short,
/// truncated with a length otherwise. Console formatting shared by
/// the CLI entry points (not a renderer: stdout has no artifact).
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
    fn summarize_counts_in_fixed_order() {
        use super::super::results::CaseOutcome;
        let outcomes = vec![
            CaseOutcome::Skipped {
                case_id: "s".to_string(),
                category: super::super::manifest::RomCategory::Cpu,
                description: String::new(),
                rom: String::new(),
                reason: String::new(),
            },
            CaseOutcome::InternalError {
                case_id: "e".to_string(),
                category: super::super::manifest::RomCategory::Ppu,
                description: String::new(),
                rom: String::new(),
                message: String::new(),
            },
        ];
        let summary = summarize(&outcomes);
        assert_eq!((summary.passed, summary.failed, summary.ignored), (0, 1, 1));
        assert_eq!(summary.expected_failed, 0);
        assert_eq!(summarize(&[]).failed, 0);
    }

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

    #[test]
    fn suite_name_prefers_second_segment() {
        assert_eq!(
            suite_name("nes-test-roms/blargg_apu_2005.07.30/01.len_ctr.nes"),
            "blargg_apu_2005.07.30"
        );
        assert_eq!(
            suite_name("nes-test-roms/cpu_interrupts_v2/rom_singles/1-cli_latency.nes"),
            "cpu_interrupts_v2"
        );
        assert_eq!(
            suite_name("gbc/aappleby_gbmicrotest/x.gb"),
            "aappleby_gbmicrotest"
        );
        assert_eq!(suite_name("lonely.nes"), "lonely.nes");
    }
}
