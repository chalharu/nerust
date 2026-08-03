use std::path::PathBuf;

use clap::{Parser, ValueEnum};

use nerust_gbc_rom_test::{
    manifest::{GbcModel, RomManifest},
    report::{Summary, write_html_report, write_json},
    runner::run_manifest,
};

#[derive(Parser)]
#[command(name = "gbc-rom-test", about = "GBC ROM test runner")]
struct Cli {
    /// Path to rom_tests.yaml
    #[arg(short, long, default_value = "rom_tests.yaml")]
    manifest: PathBuf,

    /// Filter by test case IDs (`id` or `id@model`)
    ids: Vec<String>,

    /// Filter by hardware model (dmg, cgb_c, cgb_d, agb)
    #[arg(long)]
    model: Vec<GbcModel>,

    /// Filter by tag
    #[arg(long)]
    tag: Vec<String>,

    /// Output format
    #[arg(long, value_enum, default_value = "text")]
    format: OutputFormat,

    /// Write HTML report (same as --format html)
    #[arg(short, long)]
    report: bool,

    /// Open the HTML report in the default browser after tests complete
    #[arg(short = 'O', long)]
    open: bool,

    /// Ignore the manifest's expected_failures list; every failure counts
    #[arg(long)]
    ignore_expected_failures: bool,
}

#[derive(Clone, Copy, PartialEq, Eq, ValueEnum)]
enum OutputFormat {
    Text,
    Json,
    Html,
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

fn artifacts_dir() -> PathBuf {
    let target_dir = std::env::var("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("target"));
    target_dir.join("rom_tests")
}

fn failure_detail(case: &nerust_gbc_rom_test::report::CaseResult) -> String {
    if let Some(ref error) = case.error {
        return error.clone();
    }
    case.checks
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
}

fn main() {
    let cli = Cli::parse();
    let manifest_path = manifest_path(&cli);

    let manifest = RomManifest::load(&manifest_path).expect("failed to load manifest");
    let rom_root = manifest_path.parent().unwrap().join(&manifest.rom_root);

    let cells = manifest.select(&cli.ids, &cli.model, &cli.tag);
    if cells.is_empty() {
        eprintln!("No matching test cases found");
        std::process::exit(1);
    }

    let artifacts = artifacts_dir();
    let expected_failures: &[String] = if cli.ignore_expected_failures {
        &[]
    } else {
        &manifest.expected_failures
    };
    let results = run_manifest(&rom_root, &cells, Some(&artifacts), expected_failures);

    let manifest_name = manifest_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("rom_tests");

    let format = if cli.format == OutputFormat::Text && (cli.report || cli.open) {
        OutputFormat::Html
    } else {
        cli.format
    };

    match format {
        OutputFormat::Text => {
            for result in &results {
                if result.passed {
                    println!("{} ... ok", result.id);
                } else if result.expected_failure {
                    println!(
                        "{} ... expected failure ({})",
                        result.id,
                        failure_detail(result)
                    );
                } else {
                    println!("{} ... FAILED ({})", result.id, failure_detail(result));
                }
            }
        }
        OutputFormat::Json => {
            println!("{}", write_json(manifest_name, &results));
        }
        OutputFormat::Html => {
            let _ = write_html_report(None, manifest_name, &results);
        }
    }

    let summary = Summary::of(&results);
    let summary_line = if summary.expected_failures > 0 {
        format!(
            "{} passed, {} failed ({} expected, {} unexpected)",
            summary.passed, summary.failed, summary.expected_failures, summary.unexpected
        )
    } else {
        format!("{} passed, {} failed", summary.passed, summary.failed)
    };
    if format == OutputFormat::Json {
        eprintln!("{}", summary_line);
    } else {
        println!("\n{}", summary_line);
    }

    if cli.open
        && let Ok(outcome) = write_html_report(None, manifest_name, &results)
    {
        let path = &outcome.report_path;
        if open::that(path).is_ok() {
            eprintln!("Opened report: {}", path.display());
        }
    }

    if summary.unexpected > 0 {
        std::process::exit(1);
    }
}
