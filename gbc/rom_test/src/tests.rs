use std::path::Path;

use super::{error::RomTestError, manifest::RomManifest, runner::run_case};

/// Run all cases in a manifest and collect results.
pub fn run_manifest(manifest_path: &Path) -> Result<(), RomTestError> {
    let manifest = RomManifest::load(manifest_path)?;
    let rom_root = manifest
        .rom_root
        .parent()
        .map(|p| manifest_path.parent().unwrap_or(Path::new("")).join(p))
        .unwrap_or_else(|| {
            manifest_path
                .parent()
                .unwrap_or(Path::new(""))
                .join(&manifest.rom_root)
        });

    let mut passed = 0u32;
    let mut failed = 0u32;

    for case in &manifest.cases {
        print!("  {} ... ", case.id);
        match run_case(case, &rom_root, None) {
            Ok((output, _shots)) => {
                let has_passed = case.has_explicit_verification()
                    || output.contains("Passed")
                    || output.contains("PASS");
                if has_passed {
                    println!("ok");
                    passed += 1;
                } else {
                    println!("FAILED (output: {:?})", output);
                    failed += 1;
                }
            }
            Err(e) => {
                println!("ERROR: {}", e);
                failed += 1;
            }
        }
    }

    println!("\n{} passed, {} failed", passed, failed);
    if failed > 0 {
        Err(RomTestError::InvalidManifest(format!(
            "{} test(s) failed",
            failed
        )))
    } else {
        Ok(())
    }
}
