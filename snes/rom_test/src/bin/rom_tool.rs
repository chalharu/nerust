// Copyright (c) 2026 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use clap::{Arg, ArgAction, Command};
use nerust_snes_rom_test::manifest::{RomManifest, load_default_manifest, load_manifest};
use nerust_snes_rom_test::render::render_screen;
use nerust_snes_rom_test::report::{default_output_root, write_html_report};
use nerust_snes_rom_test::results::{CaseOutcome, ValidationOptions};
use nerust_snes_rom_test::runner::{load_core_for_case, validate_case_with_options};
use std::path::PathBuf;
use std::time::Instant;

const SNES_NTSC_MASTER_CLOCK_HZ: f64 = 21_477_272.0;
const SNES_MASTER_CLOCKS_PER_SCANLINE: u64 = 1364;
const SNES_SCANLINES_PER_FRAME: u64 = 262;
const CPU_MASTER_CLOCKS_PER_CYCLE: u64 = 6;

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
        .subcommand(
            Command::new("benchmark")
                .about("Benchmark selected SNES ROM cases against NTSC real time")
                .arg(
                    Arg::new("frames")
                        .long("frames")
                        .value_name("COUNT")
                        .default_value("120")
                        .value_parser(clap::value_parser!(u64)),
                )
                .arg(
                    Arg::new("no-render")
                        .long("no-render")
                        .action(ArgAction::SetTrue)
                        .help("Skip per-frame software rendering and measure core execution only"),
                )
                .arg(
                    Arg::new("threshold")
                        .long("threshold")
                        .value_name("RATIO")
                        .default_value("1.0")
                        .value_parser(clap::value_parser!(f64)),
                ),
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
        Some(("benchmark", subcommand_matches)) => run_benchmark(
            &manifest,
            &case_ids,
            *subcommand_matches
                .get_one::<u64>("frames")
                .ok_or_else(|| "missing benchmark frame count".to_string())?,
            !subcommand_matches.get_flag("no-render"),
            *subcommand_matches
                .get_one::<f64>("threshold")
                .ok_or_else(|| "missing benchmark threshold".to_string())?,
        ),
        Some(("list", _)) => run_list(&manifest, &case_ids),
        _ => Err("subcommand required: validate, benchmark, or list".to_string()),
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

fn run_benchmark(
    manifest: &RomManifest,
    case_ids: &[String],
    frames: u64,
    render_each_frame: bool,
    threshold: f64,
) -> Result<(), String> {
    if case_ids.is_empty() {
        return Err("benchmark requires at least one --case ID".to_string());
    }
    if frames == 0 {
        return Err("benchmark requires --frames > 0".to_string());
    }
    if !threshold.is_finite() || threshold <= 0.0 {
        return Err("benchmark requires --threshold > 0".to_string());
    }

    let cases = manifest
        .select(case_ids)
        .map_err(|error| error.to_string())?;
    let total = cases.len();
    let cycles_per_frame = cpu_cycles_for_frames(1);
    let mut realtime_cases = 0_usize;

    println!(
        "mode=benchmark cases={} frames={} cycles_per_frame={} render_each_frame={} threshold={:.2}x",
        total, frames, cycles_per_frame, render_each_frame, threshold
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

        let mut core = load_core_for_case(case)?;
        let start_cycles = core.master_cycles();
        let started = Instant::now();
        let mut frames_executed = 0_u64;
        for _ in 0..frames {
            if matches!(core.current_state(), nerust_snes_core::CpuState::Stopped) {
                break;
            }
            core.run_for_cycles(cycles_per_frame)
                .map_err(|error| format!("core error during benchmark: {error}"))?;
            frames_executed += 1;
            if render_each_frame {
                render_screen(&core)
                    .map_err(|error| format!("failed to render benchmark frame: {error}"))?;
            }
        }

        let wall_seconds = started.elapsed().as_secs_f64();
        let cycles_executed = core.master_cycles().saturating_sub(start_cycles);
        let emulated_seconds = emulated_seconds_for_cpu_cycles(cycles_executed);
        let realtime_ratio = if wall_seconds > 0.0 {
            emulated_seconds / wall_seconds
        } else {
            f64::INFINITY
        };
        let realtime_status = if realtime_ratio >= threshold {
            realtime_cases += 1;
            "pass"
        } else {
            "slow"
        };

        println!(
            "  status={} frames={} cycles={} emulated_seconds={:.3} wall_seconds={:.3} speed={:.2}x",
            realtime_status,
            frames_executed,
            cycles_executed,
            emulated_seconds,
            wall_seconds,
            realtime_ratio
        );
    }

    println!(
        "summary realtime={} slow={}",
        realtime_cases,
        total.saturating_sub(realtime_cases)
    );

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

fn cpu_cycles_for_frames(frames: u64) -> u64 {
    let master_clocks = u128::from(frames)
        * u128::from(SNES_SCANLINES_PER_FRAME)
        * u128::from(SNES_MASTER_CLOCKS_PER_SCANLINE);
    ((master_clocks + u128::from(CPU_MASTER_CLOCKS_PER_CYCLE / 2))
        / u128::from(CPU_MASTER_CLOCKS_PER_CYCLE)) as u64
}

fn emulated_seconds_for_cpu_cycles(cycles: u64) -> f64 {
    (cycles as f64) * (CPU_MASTER_CLOCKS_PER_CYCLE as f64) / SNES_NTSC_MASTER_CLOCK_HZ
}

#[cfg(test)]
mod tests {
    use super::{cpu_cycles_for_frames, emulated_seconds_for_cpu_cycles};

    #[test]
    fn cpu_cycles_for_frames_rounds_ntsc_master_clock_budget() {
        assert_eq!(cpu_cycles_for_frames(1), 59_561);
        assert_eq!(cpu_cycles_for_frames(3), 178_684);
    }

    #[test]
    fn emulated_seconds_use_ntsc_master_clock_rate() {
        let seconds = emulated_seconds_for_cpu_cycles(3_579_545);
        assert!((seconds - 1.0).abs() < 0.001);
    }
}
