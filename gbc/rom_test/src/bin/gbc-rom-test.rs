use std::path::PathBuf;

use clap::Parser;

use nerust_gbc_rom_test::{
    manifest::RomManifest,
    report::{write_html_report, CaseResult},
    runner::run_case,
};

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

    /// Write HTML report to $CARGO_TARGET_DIR/rom_tests/
    #[arg(short, long)]
    report: bool,
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

    let mut results: Vec<CaseResult> = Vec::new();
    let mut passed = 0u32;
    let mut failed = 0u32;

    for case in selected {
        print!("{} ... ", case.id);
        let result = match run_case(case, &rom_root) {
            Ok(output) => {
                let passed = output.contains("Passed") || output.contains("PASS");
                if passed {
                    println!("ok");
                } else {
                    println!("FAILED (output len={})", output.len());
                }
                CaseResult {
                    id: case.id.clone(),
                    category: case.category.clone(),
                    description: case.description.clone(),
                    passed,
                    output: if passed { String::new() } else { output },
                    error: None,
                    screenshots: Vec::new(),
                }
            }
            Err(e) => {
                println!("ERROR: {}", e);
                CaseResult {
                    id: case.id.clone(),
                    category: case.category.clone(),
                    description: case.description.clone(),
                    passed: false,
                    output: String::new(),
                    error: Some(e.to_string()),
                    screenshots: Vec::new(),
                }
            }
        };
        if result.passed {
            passed += 1;
        } else {
            failed += 1;
        }
        results.push(result);
    }

    println!("\n{} passed, {} failed", passed, failed);

    if cli.report {
        let manifest_name = manifest_path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("rom_tests");
        write_html_report(None, manifest_name, &results).ok();
    }

    if failed > 0 {
        std::process::exit(1);
    }
}
