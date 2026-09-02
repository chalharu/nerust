mod case_expansion;

pub mod error;
pub mod manifest;
pub mod media;
pub mod report;
pub mod runner;
pub mod verify;

#[cfg(test)]
fn run_generated_manifest_case(id: &str) {
    let manifest_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("rom_tests.yaml");
    let manifest = manifest::RomManifest::load(&manifest_path).expect("manifest must be valid");
    let rom_root = manifest_path.parent().unwrap().join(&manifest.rom_root);
    let selected = manifest.select(&[id.to_string()]);
    assert_eq!(selected.len(), 1, "generated case `{id}` must exist");
    let expected = manifest.is_expected_failure(id);
    assert_case_passed(runner::run_case(&selected[0], &rom_root, None, expected));
}

#[cfg(test)]
fn assert_case_passed(result: report::CaseResult) {
    assert!(
        result.passed,
        "{}: {}",
        result.id,
        result.error.unwrap_or_else(|| result
            .checks
            .iter()
            .filter(|check| !check.passed)
            .map(|check| format!(
                "{}: expected {}, got {}",
                check.name, check.expected, check.actual
            ))
            .collect::<Vec<_>>()
            .join("; "))
    );
}

#[cfg(test)]
include!(concat!(env!("OUT_DIR"), "/generated_rom_manifest_tests.rs"));

#[cfg(test)]
mod tests {
    #[test]
    fn bundled_manifest_is_valid() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("rom_tests.yaml");
        super::manifest::RomManifest::load(&path).unwrap();
    }

    #[test]
    #[ignore]
    fn generate_armwrestler_references() {
        let manifest_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("rom_tests.yaml");
        let manifest = super::manifest::RomManifest::load(&manifest_path).expect("manifest must be valid");
        let rom_root = manifest_path.parent().unwrap().join(&manifest.rom_root);
        let refs_dir = rom_root.join("armwrestler-gba-fixed/refs");
        std::fs::create_dir_all(&refs_dir).unwrap();
        
        let armwrestler_ids = [
            "armwrestler_arm_alu",
            "armwrestler_arm_alu_part2",
            "armwrestler_arm_ldr_str",
            "armwrestler_arm_ldm_stm",
            "armwrestler_thumb_alu",
            "armwrestler_thumb_ldr_str",
            "armwrestler_thumb_ldm_stm",
        ];
        for id in &armwrestler_ids {
            let selected = manifest.select(&[id.to_string()]);
            assert!(!selected.is_empty(), "Case {} not found", id);
            let result = super::runner::run_case(&selected[0], &rom_root, Some(&rom_root), false);
            assert!(result.error.is_none(), "Test {} failed: {:?}", id, result.error);
            if let Some(screenshot) = &result.screenshot {
                let src = rom_root.join("screenshots").join(screenshot);
                let dst = refs_dir.join(format!("{}.png", id));
                std::fs::copy(&src, &dst).unwrap();
                println!("Saved {}", id);
            } else {
                panic!("No screenshot saved for {}", id);
            }
        }
    }
}
