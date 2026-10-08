use std::{path::PathBuf, time::Instant};

use clap::{Arg, ArgAction, Command};
use nerust_rom_test::{
    manifest::{RomManifest, load_default_manifest, load_manifest},
    report::{Report, ReportRenderer, summarize},
    results::{CaseOutcome, ValidationOptions},
    runner::validate_case,
    system_factories,
};
use nerust_rom_tool::{default_output_root, hex_preview, html::HtmlReportRenderer};

pub fn main() {
    if let Err(message) = run() {
        eprintln!("{message}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let args = parse_args(std::env::args())?;
    let manifest = match args.manifest_path {
        Some(manifest_path) => load_manifest(&manifest_path).map_err(|error| error.to_string())?,
        None => load_default_manifest().map_err(|error| error.to_string())?,
    };

    match args.subcommand {
        Subcommand::Validate => run_command(
            &manifest,
            &args.case_ids,
            args.perf_only,
            ValidationOptions::report(),
            output_dir_for("validate"),
            true,
        ),
        Subcommand::Capture => run_command(
            &manifest,
            &args.case_ids,
            args.perf_only,
            ValidationOptions::capturing(),
            output_dir_for("capture"),
            false,
        ),
        Subcommand::List => {
            let mut current_category = None;
            for case in manifest
                .select(&args.case_ids, args.perf_only)
                .map_err(|error| error.to_string())?
            {
                if current_category != Some(case.category) {
                    current_category = Some(case.category);
                    println!("[{}]", case.category.label());
                }
                println!(
                    "{} rom={} perf={} description={}",
                    case.id, case.rom, case.perf, case.description
                );
            }
            Ok(())
        }
    }
}

/// Parsed `rom_tool` invocation. Kept separate from [`run`] so argument
/// handling is unit-testable without a manifest or ROMs: `run` only
/// executes an already-parsed invocation.
#[derive(Debug, PartialEq)]
struct ToolArgs {
    manifest_path: Option<PathBuf>,
    case_ids: Vec<String>,
    perf_only: bool,
    subcommand: Subcommand,
}

#[derive(Debug, PartialEq)]
enum Subcommand {
    Validate,
    Capture,
    List,
}

fn parse_args<I, S>(args: I) -> Result<ToolArgs, String>
where
    I: IntoIterator<Item = S>,
    S: Into<std::ffi::OsString> + Clone,
{
    let matches = Command::new("rom_tool")
        .about("ROM test validation and capture tooling backed by rom_test/rom_tests.yaml")
        .arg(
            Arg::new("manifest")
                .long("manifest")
                .value_name("PATH")
                .global(true),
        )
        .arg(
            Arg::new("case")
                .long("case")
                .value_name("ID")
                .action(ArgAction::Append)
                .global(true),
        )
        .arg(
            Arg::new("perf-only")
                .long("perf-only")
                .action(ArgAction::SetTrue)
                .global(true),
        )
        .subcommand(
            Command::new("validate")
                .about("Validate configured ROM cases and generate HTML output"),
        )
        .subcommand(
            Command::new("capture")
                .about("Capture actual hashes and screenshots without asserting"),
        )
        .subcommand(Command::new("list").about("List configured ROM cases"))
        .try_get_matches_from(args)
        .map_err(|error| error.to_string())?;

    let subcommand = match matches.subcommand_name() {
        Some("validate") => Subcommand::Validate,
        Some("capture") => Subcommand::Capture,
        Some("list") => Subcommand::List,
        _ => return Err("subcommand required: validate, capture, or list".to_string()),
    };

    Ok(ToolArgs {
        manifest_path: matches.get_one::<String>("manifest").map(PathBuf::from),
        case_ids: matches
            .get_many::<String>("case")
            .map(|values| values.cloned().collect::<Vec<_>>())
            .unwrap_or_default(),
        perf_only: matches.get_flag("perf-only"),
        subcommand,
    })
}

fn run_command(
    manifest: &RomManifest,
    case_ids: &[String],
    perf_only: bool,
    options: ValidationOptions,
    output_dir: PathBuf,
    fail_on_mismatch: bool,
) -> Result<(), String> {
    let cases = manifest
        .select(case_ids, perf_only)
        .map_err(|error| error.to_string())?;
    let mode = if fail_on_mismatch {
        "validate"
    } else {
        "capture"
    };
    let total = cases.len();
    let mut outcomes = Vec::with_capacity(total);
    // Construction selects the systems once; everything downstream
    // drives through `dyn CoreFactory`. Empty without features: every
    // case is then ignored, never failed.
    let factories = system_factories();
    let mut current_category = None;
    // Wall-clock baseline for CI regression tracking (Phase 3 gates on
    // this line; per-case perf stays in the `perf` binary).
    let started = Instant::now();

    println!(
        "mode={mode} cases={total} output_dir={}",
        output_dir.display()
    );

    for (index, case) in cases.into_iter().enumerate() {
        if current_category != Some(case.category) {
            current_category = Some(case.category);
            println!("[{}]", case.category.label());
        }
        println!(
            "[{}/{}] mode={} case={} target_frames={} rom={} description={}",
            index + 1,
            total,
            mode,
            case.id,
            case.final_frame(),
            case.rom,
            case.description
        );

        let outcome = validate_case(&factories, case, options);
        print_outcome(&outcome, !options.check_expectations);
        outcomes.push(outcome);
    }

    println!(
        "writing_report={} mode={} cases={total}",
        output_dir.display(),
        mode
    );
    let summary = summarize(&outcomes);
    let started_at = std::time::SystemTime::now();
    let report_path = HtmlReportRenderer
        .render(
            &Report {
                title: if fail_on_mismatch {
                    "ROM validation report"
                } else {
                    "ROM capture report"
                },
                summary: &summary,
                started_at,
                outcomes: &outcomes,
            },
            &output_dir,
        )
        .map_err(|error| error.to_string())?;

    println!(
        "report={} mode={} passed={} failed={} expected-fail={} ignored={} elapsed_secs={:.1}",
        report_path.display(),
        mode,
        summary.passed,
        summary.failed,
        summary.expected_failed,
        summary.ignored,
        started.elapsed().as_secs_f64(),
    );

    if fail_on_mismatch && summary.failed > 0 {
        return Err(format!(
            "{} ROM case(s) failed validation; see {}",
            summary.failed,
            report_path.display()
        ));
    }

    Ok(())
}

fn print_outcome(outcome: &CaseOutcome, full_bytes: bool) {
    // In capture mode the console is machine-consumed for pinning:
    // print full serial bytes (a truncated preview once caused a
    // complete suite to be misread as stalled). Validate mode keeps
    // the compact preview.
    fn serial_text(bytes: &[u8], full: bool) -> String {
        if full {
            bytes.iter().map(|b| format!("{b:02X}")).collect()
        } else {
            hex_preview(bytes)
        }
    }
    match outcome {
        CaseOutcome::Completed(validation) => {
            let status = if validation.passed() {
                "pass"
            } else if validation.is_expected_failure() {
                "expected-fail"
            } else {
                "fail"
            };
            println!(
                "case={} category={} status={} frames={} final_hash=0x{:016X}",
                outcome.case_id(),
                validation.category.label(),
                status,
                validation.frames,
                validation.final_screen_hash
            );
            println!("  description={}", validation.description);
            for check in &validation.screen_checks {
                println!(
                    "  frame={} expected=0x{:016X} actual=0x{:016X} status={}",
                    check.frame,
                    check.expected_hash,
                    check.actual_hash,
                    if check.passed() { "pass" } else { "fail" }
                );
            }
            for check in &validation.memory_checks {
                println!(
                    "  memory frame={} address=0x{:04X} expected=0x{:02X} actual=0x{:02X} expected_bus={} actual_bus={} status={}",
                    check.frame,
                    check.address,
                    check.expected_value,
                    check.actual_value,
                    if check.expected_open_bus {
                        "open-bus"
                    } else {
                        "mapped"
                    },
                    if check.actual_open_bus {
                        "open-bus"
                    } else {
                        "mapped"
                    },
                    if check.passed() { "pass" } else { "fail" }
                );
            }
            for check in &validation.register_checks {
                println!(
                    "  register frame={} name={} expected=0x{:X} actual=0x{:X} status={}",
                    check.frame,
                    check.name,
                    check.expected_value,
                    check.actual_value,
                    if check.passed() { "pass" } else { "fail" }
                );
            }
            for check in &validation.serial_checks {
                println!(
                    "  serial frame={} channel={} expected={} actual={} status={}",
                    check.frame,
                    check.channel,
                    serial_text(&check.expected_bytes, full_bytes),
                    serial_text(&check.actual_bytes, full_bytes),
                    if check.passed() { "pass" } else { "fail" }
                );
            }
            println!(
                "  audio sample_rate={} samples={} hash=0x{:016X}",
                validation.audio.sample_rate, validation.audio.samples, validation.audio.hash
            );
            for check in &validation.log_checks {
                println!(
                    "  log frame={} channel={} end={} fails=[{}] missing=[{}] unexpected=[{}] status={}",
                    check.frame,
                    check.channel,
                    check.expected_end,
                    check.fail_names.join(", "),
                    check.missing_allowed.join(", "),
                    check.unexpected_fail.join(", "),
                    if check.passed() { "pass" } else { "fail" }
                );
            }
            for failure in &validation.failures {
                println!("  failure={failure}");
            }
            for note in &validation.stale_notes {
                println!("  stale={note}");
            }
        }
        CaseOutcome::InternalError {
            case_id,
            category,
            description,
            rom,
            message,
        } => {
            println!(
                "case={case_id} category={} status=error rom={rom} description={} message={message}",
                category.label(),
                description
            );
        }
        CaseOutcome::Skipped {
            case_id,
            category,
            description,
            rom,
            reason,
        } => {
            println!(
                "case={case_id} category={} status=ignored rom={rom} description={} reason={reason}",
                category.label(),
                description
            );
        }
    }
}

fn output_dir_for(name: &str) -> PathBuf {
    default_output_root().join(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_args_dispatches_subcommands() {
        let validate = parse_args(["rom_tool", "validate"]).expect("validate");
        assert_eq!(validate.subcommand, Subcommand::Validate);
        assert_eq!(validate.manifest_path, None);
        assert!(validate.case_ids.is_empty());
        assert!(!validate.perf_only);

        let capture = parse_args([
            "rom_tool",
            "--manifest",
            "custom.yaml",
            "--case",
            "a",
            "--case",
            "b",
            "--perf-only",
            "capture",
        ])
        .expect("capture");
        assert_eq!(capture.subcommand, Subcommand::Capture);
        assert_eq!(capture.manifest_path, Some(PathBuf::from("custom.yaml")));
        assert_eq!(capture.case_ids, vec!["a".to_string(), "b".to_string()]);
        assert!(capture.perf_only);

        let list = parse_args(["rom_tool", "list"]).expect("list");
        assert_eq!(list.subcommand, Subcommand::List);
    }

    #[test]
    fn parse_args_requires_a_subcommand() {
        assert!(parse_args(["rom_tool"]).is_err());
        assert!(parse_args(["rom_tool", "bogus"]).is_err());
    }
}
