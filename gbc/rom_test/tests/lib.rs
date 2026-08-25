use std::path::Path;

use nerust_gbc_rom_test::{manifest::RomManifest, report::Summary, runner::run_manifest};

/// Test all ROM matrix cells defined in the YAML manifest.
///
/// Cells declared in `expected_failures` may fail; anything else failing,
/// or an expected failure that now passes, fails this test.
#[test]
fn rom_tests() {
    let manifest_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("rom_tests.yaml");
    let manifest = RomManifest::load(&manifest_path).expect("failed to load manifest");
    let rom_root = manifest_path.parent().unwrap().join(&manifest.rom_root);

    let cells = manifest.select(&[], &[], &[]);
    assert!(!cells.is_empty(), "manifest must select at least one cell");
    let results = run_manifest(&rom_root, &cells, None, &manifest.expected_failures);

    for result in &results {
        if result.passed {
            println!("{} ... ok", result.id);
        } else if result.expected_failure {
            println!("{} ... expected failure", result.id);
        } else {
            println!("{} ... FAILED", result.id);
            for check in result.checks.iter().filter(|check| !check.passed) {
                eprintln!(
                    "  {}: expected {}, got {}",
                    check.name, check.expected, check.actual
                );
            }
            if let Some(ref error) = result.error {
                eprintln!("  error: {}", error);
            }
        }
    }

    let summary = Summary::of(&results);
    for result in results.iter().filter(|r| r.unexpected()) {
        if result.expected_failure {
            eprintln!(
                "unexpected pass: {} (remove from expected_failures)",
                result.id
            );
        } else {
            eprintln!("unexpected failure: {}", result.id);
        }
    }
    eprintln!("{} passed, {} failed", summary.passed, summary.failed);
    assert_eq!(
        summary.unexpected, 0,
        "{} unexpected result(s)",
        summary.unexpected
    );
}
