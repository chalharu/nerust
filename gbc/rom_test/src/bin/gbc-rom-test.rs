use std::path::PathBuf;

use clap::Parser;

use nerust_gbc_rom_test::{
    manifest::RomManifest,
    report::{CaseResult, write_html_report},
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

    /// Open the HTML report in the default browser after tests complete
    #[arg(short = 'O', long)]
    open: bool,
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

    // Determine screenshots directory if --report is enabled
    // If --open is specified without --report, enable report implicitly
    let do_report = cli.report || cli.open;
    let screenshots_dir = if do_report {
        let target_dir = std::env::var("CARGO_TARGET_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("target"));
        let dir = target_dir.join("rom_tests").join("screenshots");
        std::fs::create_dir_all(&dir).ok();
        Some(dir)
    } else {
        None
    };

    for case in selected {
        print!("{} ... ", case.id);
        let result = match run_case(case, &rom_root, screenshots_dir.as_deref()) {
            Ok((output, shots)) => {
                // Pass if: serial says PASS, OR frame hash matched (no error but no serial)
                let passed = output.contains("Passed") || output.contains("PASS")
                    || output.is_empty();
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
                    screenshots: shots,
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

    let mut report_summary = None;
    if do_report {
        let manifest_name = manifest_path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("rom_tests");
        report_summary = write_html_report(None, manifest_name, &results).ok();
    }

    if cli.open && let Some(ref summary) = report_summary {
            let path = &summary.report_path;
            if open::that(path).is_ok() {
                eprintln!("Opened report: {}", path.display());
            }
    }

    if failed > 0 {
        std::process::exit(1);
    }
}
