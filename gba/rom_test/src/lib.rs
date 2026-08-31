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
    let mut result = runner::run_case(&selected[0], &rom_root);
    result.expected_failure = manifest.is_expected_failure(id);
    assert_case_expected(result);
}

#[cfg(test)]
fn assert_case_expected(result: report::CaseResult) {
    assert!(
        !result.unexpected(),
        "{}: {}",
        result.id,
        if result.passed {
            "unexpected pass; remove it from expected_failures".to_string()
        } else {
            result.error.unwrap_or_else(|| {
                result
                    .checks
                    .iter()
                    .filter(|check| !check.passed)
                    .map(|check| {
                        format!(
                            "{}: expected {}, got {}",
                            check.name, check.expected, check.actual
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("; ")
            })
        }
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
}
