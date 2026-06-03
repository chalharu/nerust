use nerust_snes_rom_test::manifest::{RomManifest, load_default_manifest};
use nerust_snes_rom_test::results::CaseOutcome;
use nerust_snes_rom_test::runner::validate_case;
use std::sync::OnceLock;

#[test]
fn rom_manifest_is_well_formed() {
    let manifest = manifest();
    assert_eq!(
        GENERATED_ROM_CASE_COUNT,
        manifest.cases.len(),
        "generated test count should match the manifest case count"
    );
}

fn manifest() -> &'static RomManifest {
    static MANIFEST: OnceLock<RomManifest> = OnceLock::new();
    MANIFEST.get_or_init(|| load_default_manifest().expect("ROM manifest should load"))
}

fn run_generated_manifest_case(case_id: &str) {
    let case = manifest()
        .case(case_id)
        .unwrap_or_else(|| panic!("ROM case `{case_id}` should exist in the manifest"));
    let outcome = validate_case(case);

    match outcome {
        CaseOutcome::Completed(validation) if validation.passed() => {}
        CaseOutcome::Completed(validation) => {
            panic!(
                "{} ({} steps, final screen hash 0x{:016X}):\n{}",
                validation.case_id,
                validation.steps_executed,
                validation.final_screen_hash,
                validation.failures.join("\n")
            );
        }
        CaseOutcome::InternalError {
            case_id, message, ..
        } => {
            panic!("{case_id}: {message}");
        }
    }
}

include!(concat!(env!("OUT_DIR"), "/generated_rom_manifest_tests.rs"));
