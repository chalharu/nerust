use std::path::Path;

use nerust_gbc_rom_test::{manifest::RomManifest, runner::run_case};

/// Test all ROM cases defined in the YAML manifest.
#[test]
fn rom_tests() {
    let manifest_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("rom_tests.yaml");
    let manifest = RomManifest::load(&manifest_path).expect("failed to load manifest");
    let rom_root = manifest_path.parent().unwrap().join(&manifest.rom_root);

    let mut passed = 0u32;
    let mut failed = 0u32;

    for case in &manifest.cases {
        print!("{} ... ", case.id);
        match run_case(case, &rom_root, None) {
            Ok((output, _shots)) => {
                let ok = output.contains("Passed");
                if ok {
                    println!("ok");
                    passed += 1;
                } else {
                    println!("FAILED (output: {:?})", output);
                    failed += 1;
                }
            }
            Err(e) => {
                println!("ERROR");
                eprintln!("  {}", e);
                failed += 1;
            }
        }
    }

    assert_eq!(failed, 0, "{} test(s) failed", failed);
    eprintln!("{} passed, {} failed", passed, failed);
}
