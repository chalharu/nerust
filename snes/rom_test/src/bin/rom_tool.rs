// Copyright (c) 2026 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use clap::{Arg, ArgAction, Command};
use nerust_snes_rom_test::manifest::{RomManifest, load_default_manifest, load_manifest};
use nerust_snes_rom_test::report::{default_output_root, write_html_report};
use nerust_snes_rom_test::results::{CaseOutcome, ValidationOptions};
use nerust_snes_rom_test::runner::validate_case_with_options;
use std::path::PathBuf;

fn main() {
    if let Err(message) = run() {
        eprintln!("{message}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let matches = Command::new("rom_tool")
        .about("SNES ROM test validation and HTML capture tooling")
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
        .subcommand(
            Command::new("validate")
                .about("Validate configured SNES ROM cases and generate an HTML report")
                .arg(Arg::new("output-dir").long("output-dir").value_name("DIR")),
        )
        .subcommand(Command::new("list").about("List configured SNES ROM cases"))
        .get_matches();

    let manifest = matches
        .get_one::<String>("manifest")
        .map(PathBuf::from)
        .map_or_else(
            || load_default_manifest().map_err(|error| error.to_string()),
            |path| load_manifest(&path).map_err(|error| error.to_string()),
        )?;
    let case_ids = matches
        .get_many::<String>("case")
        .map(|values| values.cloned().collect::<Vec<_>>())
        .unwrap_or_default();

    match matches.subcommand() {
        Some(("validate", subcommand_matches)) => run_validate(
            &manifest,
            &case_ids,
            subcommand_matches
                .get_one::<String>("output-dir")
                .map(PathBuf::from)
                .unwrap_or_else(|| default_output_root().join("validate")),
        ),
        Some(("list", _)) => run_list(&manifest, &case_ids),
        _ => Err("subcommand required: validate or list".to_string()),
    }
}

fn run_list(manifest: &RomManifest, case_ids: &[String]) -> Result<(), String> {
    for case in manifest
        .select(case_ids)
        .map_err(|error| error.to_string())?
    {
        println!(
            "{} rom={} max_steps={} description={}",
            case.id,
            case.rom.display(),
            case.max_steps,
            case.description
        );
    }
    Ok(())
}

fn run_validate(
    manifest: &RomManifest,
    case_ids: &[String],
    output_dir: PathBuf,
) -> Result<(), String> {
    let cases = manifest
        .select(case_ids)
        .map_err(|error| error.to_string())?;
    let total = cases.len();
    let mut outcomes = Vec::with_capacity(total);

    println!(
        "mode=validate cases={} output_dir={}",
        total,
        output_dir.display()
    );

    for (index, case) in cases.into_iter().enumerate() {
        println!(
            "[{}/{}] case={} rom={} description={}",
            index + 1,
            total,
            case.id,
            case.rom_path().display(),
            case.description
        );
        let outcome = validate_case_with_options(case, ValidationOptions::report());
        print_outcome(&outcome);
        outcomes.push(outcome);
    }

    let summary = write_html_report(&output_dir, "SNES ROM validation report", &outcomes)?;
    println!(
        "report={} passed={} failed={}",
        summary.report_path.display(),
        summary.passed,
        summary.failed
    );

    if summary.failed > 0 {
        return Err(format!(
            "{} SNES ROM case(s) failed validation; see {}",
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
                "  status={} steps={} final_hash=0x{:016X}",
                if validation.passed() { "pass" } else { "fail" },
                validation.steps_executed,
                validation.final_screen_hash
            );
            for failure in &validation.failures {
                println!("  failure={failure}");
            }
        }
        CaseOutcome::InternalError {
            case_id, message, ..
        } => {
            println!("  case={case_id} status=error message={message}");
        }
    }
}
