use std::path::PathBuf;

use clap::Parser;

use nerust_gbc_rom_test::{manifest::RomManifest, runner::run_case};

#[derive(Parser)]
#[command(name = "gbc-rom-test", about = "GBC ROM test runner")]
struct Cli {
    /// Path to rom_tests.yaml
    #[arg(short, long, default_value = "rom_tests.yaml")]
    manifest: PathBuf,

    /// Filter by test case IDs
    ids: Vec<String>,

    /// Performance test mode
    #[arg(short, long)]
    perf: bool,
}

fn manifest_path(cli: &Cli) -> PathBuf {
    if cli.manifest.is_absolute() {
        return cli.manifest.clone();
    }
    // Search relative to current directory first, then fall back to CARGO_MANIFEST_DIR
    let cwd = std::env::current_dir().unwrap();
    let cwd_path = cwd.join(&cli.manifest);
    if cwd_path.exists() {
        return cwd_path;
    }
    let crate_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    crate_dir.join(&cli.manifest)
}

fn main() {
    let cli = Cli::parse();
    let manifest_path = manifest_path(&cli);

    let manifest = RomManifest::load(&manifest_path).expect("failed to load manifest");
    let rom_root = manifest_path.parent().unwrap().join(&manifest.rom_root);

    let selected = manifest.select(&cli.ids, cli.perf);
    if selected.is_empty() {
        eprintln!("No matching test cases found");
        std::process::exit(1);
    }

    let mut passed = 0u32;
    let mut failed = 0u32;

    for case in selected {
        print!("{} ... ", case.id);
        match run_case(case, &rom_root) {
            Ok(output) => {
                let ok = output.contains("Passed") || output.contains("PASS");
                if ok {
                    println!("ok");
                    passed += 1;
                } else {
                    println!("FAILED (output len={})", output.len());
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
        std::process::exit(1);
    }
}
