use std::path::PathBuf;

use clap::{Parser, ValueEnum};
use nerust_gba_rom_test::{
    manifest::RomManifest,
    report::{Summary, write_json},
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
}

#[derive(Clone, Copy, ValueEnum)]
enum OutputFormat {
    Text,
    Json,
}

fn main() {
    let cli = Cli::parse();
    let manifest_path = resolve_manifest(&cli.manifest);
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
    let results = run_manifest(&rom_root, &cases);
    match cli.format {
        OutputFormat::Json => println!("{}", write_json(&results)),
        OutputFormat::Text => {
            for result in &results {
                if result.passed {
                    println!(
                        "{} ... ok ({} T-cycles)",
                        result.id, result.executed_tcycles
                    );
                } else {
                    let detail = result.error.clone().unwrap_or_else(|| {
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
                    });
                    println!("{} ... FAILED ({detail})", result.id);
                }
            }
        }
    }
    let summary = Summary::of(&results);
    if matches!(cli.format, OutputFormat::Text) {
        println!("\n{} passed, {} failed", summary.passed, summary.failed);
    }
    if summary.failed > 0 {
        std::process::exit(1);
    }
}

fn resolve_manifest(path: &std::path::Path) -> PathBuf {
    if path.is_absolute() || path.exists() {
        return path.to_path_buf();
    }
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(path)
}
