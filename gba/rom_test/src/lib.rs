pub mod error;
pub mod manifest;
pub mod media;
pub mod report;
pub mod runner;
pub mod verify;

#[cfg(test)]
fn run_available_manifest_roms() {
    let manifest_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("rom_tests.yaml");
    let manifest = manifest::RomManifest::load(&manifest_path).expect("manifest must be valid");
    let rom_root = manifest_path.parent().unwrap().join(&manifest.rom_root);
    let available = manifest
        .select(&[])
        .into_iter()
        .filter(|selected| {
            rom_root
                .join(&selected.suite.name)
                .join(&selected.case.rom)
                .is_file()
        })
        .collect::<Vec<_>>();
    if available.is_empty() {
        eprintln!("no external GBA ROM assets are available; synthetic runner tests remain active");
    }
    for result in runner::run_manifest(&rom_root, &available) {
        assert_case_passed(result);
    }
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
}
