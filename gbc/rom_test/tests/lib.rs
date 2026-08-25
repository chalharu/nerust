use std::{fmt::Write as _, path::Path, sync::OnceLock};

use nerust_gbc_rom_test::{manifest::RomManifest, runner::run_manifest};

#[test]
fn rom_manifest_is_well_formed() {
    let manifest = manifest();
    let case_count = manifest
        .suites
        .iter()
        .map(|suite| suite.cases.len())
        .sum::<usize>();
    assert_eq!(
        GENERATED_ROM_CASE_COUNT, case_count,
        "generated test count should match the manifest case count"
    );
}

fn manifest() -> &'static RomManifest {
    static MANIFEST: OnceLock<RomManifest> = OnceLock::new();
    MANIFEST.get_or_init(|| {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("rom_tests.yaml");
        RomManifest::load(&path).expect("ROM manifest should load")
    })
}

fn run_generated_manifest_case(case_id: &str) {
    let manifest = manifest();
    let manifest_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("rom_tests.yaml");
    let rom_root = manifest_path.parent().unwrap().join(&manifest.rom_root);
    let cells = manifest.select(&[case_id.to_string()], &[], &[]);
    assert!(!cells.is_empty(), "ROM case `{case_id}` should exist");

    let results = run_manifest(&rom_root, &cells, None, &manifest.expected_failures);
    let mut failures = String::new();
    for result in results.iter().filter(|result| result.unexpected()) {
        if result.expected_failure {
            writeln!(
                failures,
                "{} unexpectedly passed; remove it from expected_failures",
                result.id
            )
            .unwrap();
            continue;
        }
        writeln!(failures, "{} failed", result.id).unwrap();
        for check in result.checks.iter().filter(|check| !check.passed) {
            writeln!(
                failures,
                "  {}: expected {}, got {}",
                check.name, check.expected, check.actual
            )
            .unwrap();
        }
        if let Some(error) = &result.error {
            writeln!(failures, "  error: {error}").unwrap();
        }
    }

    assert!(failures.is_empty(), "{failures}");
}

include!(concat!(env!("OUT_DIR"), "/generated_rom_manifest_tests.rs"));
