use std::{ffi::OsStr, path::PathBuf};

use clap::{Parser, ValueEnum};

use nerust_gba_rom_test::{
    manifest::RomManifest,
    report::{Summary, write_html_report, write_json},
    runner::run_manifest,
};

#[derive(Parser)]
#[command(name = "gba-rom-test", about = "Run GBA ROM tests headlessly")]
struct Cli {
    #[arg(short, long, default_value = "rom_tests.yaml")]
    manifest: PathBuf,
    /// Optional case IDs. Omit to run every case.
    ids: Vec<String>,
    #[arg(long, value_enum, default_value = "text")]
    format: OutputFormat,
    /// Write HTML report (same as --format html)
    #[arg(short, long)]
    report: bool,
    /// Open the HTML report in the default browser after tests complete
    #[arg(short = 'O', long)]
    open: bool,
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

fn failure_detail(case: &nerust_gba_rom_test::report::CaseResult) -> String {
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

    let manifest = match RomManifest::load(&manifest_path) {
        Ok(manifest) => manifest,
        Err(error) => {
            eprintln!("failed to load {}: {error}", manifest_path.display());
            std::process::exit(2);
        }
    };

    let cases = manifest.select(&cli.ids);
    if cases.is_empty() {
        eprintln!("no matching ROM test cases");
        std::process::exit(2);
    }

    let rom_root = manifest_path.parent().unwrap().join(&manifest.rom_root);
    let artifacts = artifacts_dir();
    let results = run_manifest(
        &rom_root,
        &cases,
        Some(&artifacts),
        &manifest.expected_failures,
    );

    let manifest_name = manifest_path
        .file_stem()
        .and_then(OsStr::to_str)
        .unwrap_or("rom_tests");

    let format = if cli.format == OutputFormat::Text && (cli.report || cli.open) {
        OutputFormat::Html
    } else {
        cli.format
    };

    print_results(&results, format, manifest_name);

    let summary = Summary::of(&results);
    print_summary(&summary, format);

    if cli.open {
        open_report_if_available(manifest_name, &results);
    }

    if summary.unexpected > 0 {
        std::process::exit(1);
    }
}

fn print_results(
    results: &[nerust_gba_rom_test::report::CaseResult],
    format: OutputFormat,
    manifest_name: &str,
) {
    match format {
        OutputFormat::Text => {
            for result in results {
                if result.passed {
                    println!(
                        "{} ... ok ({} T-cycles)",
                        result.id, result.executed_tcycles
                    );
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
            println!("{}", write_json(manifest_name, results));
        }
        OutputFormat::Html => {
            let _ = write_html_report(None, manifest_name, results);
        }
    }
}

fn print_summary(summary: &Summary, format: OutputFormat) {
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
}

fn open_report_if_available(
    manifest_name: &str,
    results: &[nerust_gba_rom_test::report::CaseResult],
) {
    if let Ok(outcome) = write_html_report(None, manifest_name, results) {
        let path = &outcome.report_path;
        if open::that(path).is_ok() {
            eprintln!("Opened report: {}", path.display());
        }
    }
}
