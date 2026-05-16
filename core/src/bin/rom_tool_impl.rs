// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

#[path = "../rom_test.rs"]
mod rom_test;

use clap::{Arg, ArgAction, ArgMatches, Command};
use rom_test::{
    CaseOutcome, ValidationOptions, default_output_root, load_default_manifest, load_manifest,
    validate_case, write_html_report,
};
use std::path::PathBuf;

pub fn main() {
    if let Err(message) = run() {
        eprintln!("{message}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let matches = Command::new("rom_tool")
        .about("ROM test validation and capture tooling backed by core/rom_tests.yaml")
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
        .get_matches();

    let manifest = matches
        .get_one::<String>("manifest")
        .map(PathBuf::from)
        .map_or_else(
            || load_default_manifest().map_err(|error| error.to_string()),
            |manifest_path| load_manifest(&manifest_path).map_err(|error| error.to_string()),
        )?;
    let case_ids = matches
        .get_many::<String>("case")
        .map(|values| values.cloned().collect::<Vec<_>>())
        .unwrap_or_default();
    let perf_only = matches.get_flag("perf-only");

    match matches.subcommand() {
        Some(("validate", subcommand_matches)) => run_command(
            &manifest,
            &case_ids,
            perf_only,
            ValidationOptions::report(),
            output_dir_for(subcommand_matches, "validate"),
            true,
        ),
        Some(("capture", subcommand_matches)) => run_command(
            &manifest,
            &case_ids,
            perf_only,
            ValidationOptions::capturing(),
            output_dir_for(subcommand_matches, "capture"),
            false,
        ),
        Some(("list", _)) => {
            let mut current_category = None;
            for case in manifest
                .select(&case_ids, perf_only)
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
        _ => Err("subcommand required: validate, capture, or list".to_string()),
    }
}

fn run_command(
    manifest: &rom_test::RomManifest,
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
    let mut current_category = None;

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

        let outcome = validate_case(case, options);
        print_outcome(&outcome);
        outcomes.push(outcome);
    }

    println!(
        "writing_report={} mode={} cases={total}",
        output_dir.display(),
        mode
    );
    let summary = write_html_report(
        &output_dir,
        if fail_on_mismatch {
            "ROM validation report"
        } else {
            "ROM capture report"
        },
        &outcomes,
    )
    .map_err(|error| error.to_string())?;

    println!(
        "report={} mode={} passed={} failed={}",
        summary.report_path.display(),
        mode,
        summary.passed,
        summary.failed
    );

    if fail_on_mismatch && summary.failed > 0 {
        return Err(format!(
            "{} ROM case(s) failed validation; see {}",
            summary.failed,
            summary.report_path.display()
        ));
    }

    Ok(())
}

fn print_outcome(outcome: &CaseOutcome) {
    match outcome {
        CaseOutcome::Completed(validation) => {
            println!(
                "case={} category={} status={} frames={} steps={} final_hash=0x{:016X}",
                outcome.case_id(),
                validation.category.label(),
                if validation.passed() { "pass" } else { "fail" },
                validation.frames,
                validation.steps,
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
            for check in &validation.work_ram_checks {
                println!(
                    "  work_ram frame={} address=0x{:04X} expected=0x{:02X} actual=0x{:02X} status={}",
                    check.frame,
                    check.address,
                    check.expected_value,
                    check.actual_value,
                    if check.passed() { "pass" } else { "fail" }
                );
            }
            println!(
                "  audio sample_rate={} samples={} hash=0x{:016X}",
                validation.audio.sample_rate, validation.audio.samples, validation.audio.hash
            );
            for failure in &validation.failures {
                println!("  failure={failure}");
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
    }
}

fn output_dir_for(_matches: &ArgMatches, name: &str) -> PathBuf {
    default_output_root().join(name)
}
