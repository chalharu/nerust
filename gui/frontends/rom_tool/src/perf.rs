use std::time::{Duration, Instant};

use clap::{Arg, ArgAction, Command};

use nerust_rom_test::{
    manifest::{load_default_manifest, read_rom},
    results::{CaseOutcome, ValidationOptions},
    runner::{measure_case, validate_case},
    system_factories,
};

pub fn run_cli() {
    if let Err(message) = run() {
        eprintln!("{message}");
        std::process::exit(1);
    }
}

/// Parsed `perf` invocation. Kept separate from [`run`] so argument
/// handling is unit-testable without touching ROMs or the runner:
/// `run` only executes an already-validated config.
#[derive(Debug, PartialEq)]
struct PerfArgs {
    rounds: usize,
    warmup_rounds: usize,
    case_ids: Vec<String>,
}

fn parse_args<I, S>(args: I) -> Result<PerfArgs, String>
where
    I: IntoIterator<Item = S>,
    S: Into<std::ffi::OsString> + Clone,
{
    let matches = Command::new("perf")
        .about("Benchmark perf-enabled ROM test cases from rom_test/rom_tests.yaml")
        .arg(Arg::new("rounds").long("rounds").value_name("N"))
        .arg(
            Arg::new("warmup-rounds")
                .long("warmup-rounds")
                .value_name("N"),
        )
        .arg(
            Arg::new("case")
                .long("case")
                .value_name("ID")
                .action(ArgAction::Append),
        )
        .try_get_matches_from(args)
        .map_err(|error| error.to_string())?;

    let rounds = matches
        .get_one::<String>("rounds")
        .map(String::as_str)
        .unwrap_or("5")
        .parse::<usize>()
        .map_err(|error| format!("invalid --rounds value: {error}"))?;
    if rounds == 0 {
        return Err("--rounds must be greater than 0".to_string());
    }

    let warmup_rounds = matches
        .get_one::<String>("warmup-rounds")
        .map(String::as_str)
        .unwrap_or("1")
        .parse::<usize>()
        .map_err(|error| format!("invalid --warmup-rounds value: {error}"))?;

    let case_ids = matches
        .get_many::<String>("case")
        .map(|values| values.cloned().collect::<Vec<_>>())
        .unwrap_or_default();

    Ok(PerfArgs {
        rounds,
        warmup_rounds,
        case_ids,
    })
}

fn run() -> Result<(), String> {
    let args = parse_args(std::env::args())?;
    let PerfArgs {
        rounds,
        warmup_rounds,
        case_ids,
    } = args;

    let manifest = load_default_manifest().map_err(|error| error.to_string())?;
    let cases = manifest
        .select(&case_ids, true)
        .map_err(|error| error.to_string())?;

    println!(
        "perf-suite rounds={} warmup_rounds={} cases={}",
        rounds,
        warmup_rounds,
        cases.len()
    );

    let mut roms = Vec::with_capacity(cases.len());
    // Construction selects the systems once; everything downstream
    // drives through `dyn CoreFactory`. Without systems every case is
    // ignored and the suite ends empty but successful.
    let factories = system_factories();
    for case in &cases {
        match validate_case(
            &factories,
            case,
            ValidationOptions {
                capture_screenshots: false,
                check_expectations: true,
            },
        ) {
            CaseOutcome::Completed(validation) if validation.passed() => {
                println!(
                    "validated case={} frames={} final_hash=0x{:016X} audio_samples={} audio_hash=0x{:016X}",
                    validation.case_id,
                    validation.frames,
                    validation.final_screen_hash,
                    validation.audio.samples,
                    validation.audio.hash
                );
            }
            CaseOutcome::Completed(validation) => {
                return Err(format!(
                    "validation failed for {}:\n{}",
                    validation.case_id,
                    validation.failures.join("\n")
                ));
            }
            CaseOutcome::InternalError {
                case_id, message, ..
            } => {
                return Err(format!("validation errored for {case_id}: {message}"));
            }
            CaseOutcome::Skipped {
                case_id, reason, ..
            } => {
                println!("ignored case={case_id} reason={reason}");
                continue;
            }
        }

        roms.push(
            read_rom(case)
                .map_err(|error| error.to_string())
                .map(|bytes| (*case, bytes))?,
        );
    }

    if roms.is_empty() {
        println!("perf-suite: no applicable cases, nothing to benchmark");
        return Ok(());
    }

    for _ in 0..warmup_rounds {
        for (case, rom_bytes) in &roms {
            let result =
                measure_case(&factories, case, rom_bytes).map_err(|error| error.to_string())?;
            std::hint::black_box(result.final_marker);
        }
    }

    let mut suite = Aggregate::default();
    for (case, rom_bytes) in &roms {
        let mut aggregate = Aggregate::default();
        let mut final_marker = 0_u64;

        for round in 0..rounds {
            let wall_started = Instant::now();
            let cpu_started_nanos = process_cpu_time_nanos()?;
            let result =
                measure_case(&factories, case, rom_bytes).map_err(|error| error.to_string())?;
            let wall_duration_secs = wall_started.elapsed().as_secs_f64();
            let cpu_duration_secs =
                Duration::from_nanos(process_cpu_time_nanos()?.saturating_sub(cpu_started_nanos))
                    .as_secs_f64();

            final_marker = result.final_marker;
            aggregate.last_wall_duration_secs = wall_duration_secs;
            aggregate.last_cpu_duration_secs = cpu_duration_secs;
            aggregate.total_wall_duration_secs += wall_duration_secs;
            aggregate.total_cpu_duration_secs += cpu_duration_secs;
            aggregate.total_steps += result.steps;
            aggregate.total_frames += result.frames;

            println!(
                "run round={} case={} cpu_time_ms={:.3} wall_time_ms={:.3} frames={} steps={} steps_per_cpu_sec={:.3} steps_per_wall_sec={:.3}",
                round + 1,
                case.id,
                aggregate.last_cpu_ms(),
                aggregate.last_wall_ms(),
                result.frames,
                result.steps,
                result.steps as f64 / aggregate.last_cpu_secs(),
                result.steps as f64 / aggregate.last_wall_secs(),
            );
        }

        suite.total_wall_duration_secs += aggregate.total_wall_duration_secs;
        suite.total_cpu_duration_secs += aggregate.total_cpu_duration_secs;
        suite.total_steps += aggregate.total_steps;
        suite.total_frames += aggregate.total_frames;

        let avg_wall_duration_secs = aggregate.total_wall_duration_secs / rounds as f64;
        let avg_cpu_duration_secs = aggregate.total_cpu_duration_secs / rounds as f64;
        let avg_steps = aggregate.total_steps as f64 / rounds as f64;
        let avg_frames = aggregate.total_frames as f64 / rounds as f64;

        println!(
            "summary case={} avg_cpu_time_ms={:.3} avg_wall_time_ms={:.3} avg_frames={:.1} avg_steps={:.1} avg_steps_per_cpu_sec={:.3} avg_steps_per_wall_sec={:.3} avg_frames_per_cpu_sec={:.3} final_marker=0x{final_marker:016X}",
            case.id,
            avg_cpu_duration_secs * 1_000.0,
            avg_wall_duration_secs * 1_000.0,
            avg_frames,
            avg_steps,
            avg_steps / avg_cpu_duration_secs,
            avg_steps / avg_wall_duration_secs,
            avg_frames / avg_cpu_duration_secs,
        );
    }

    let suite_avg_wall_duration_secs = suite.total_wall_duration_secs / rounds as f64;
    let suite_avg_cpu_duration_secs = suite.total_cpu_duration_secs / rounds as f64;
    let suite_avg_steps = suite.total_steps as f64 / rounds as f64;
    let suite_avg_frames = suite.total_frames as f64 / rounds as f64;
    let peak_rss_mib =
        peak_rss_mib().map_or_else(|| "n/a".to_string(), |value| format!("{value:.3}"));

    println!(
        "suite avg_cpu_time_ms={:.3} avg_wall_time_ms={:.3} avg_steps={:.1} avg_frames={:.1} avg_steps_per_cpu_sec={:.3} avg_steps_per_wall_sec={:.3} avg_frames_per_cpu_sec={:.3} peak_rss_mib={peak_rss_mib}",
        suite_avg_cpu_duration_secs * 1_000.0,
        suite_avg_wall_duration_secs * 1_000.0,
        suite_avg_steps,
        suite_avg_frames,
        suite_avg_steps / suite_avg_cpu_duration_secs,
        suite_avg_steps / suite_avg_wall_duration_secs,
        suite_avg_frames / suite_avg_cpu_duration_secs,
    );

    Ok(())
}

#[derive(Default)]
struct Aggregate {
    total_wall_duration_secs: f64,
    total_cpu_duration_secs: f64,
    total_steps: u64,
    total_frames: u64,
    last_wall_duration_secs: f64,
    last_cpu_duration_secs: f64,
}

impl Aggregate {
    fn last_wall_ms(&self) -> f64 {
        self.last_wall_duration_secs * 1_000.0
    }

    fn last_cpu_ms(&self) -> f64 {
        self.last_cpu_duration_secs * 1_000.0
    }

    fn last_wall_secs(&self) -> f64 {
        self.last_wall_duration_secs
    }

    fn last_cpu_secs(&self) -> f64 {
        self.last_cpu_duration_secs
    }
}

/// Benchmark driving lives in the engine (`measure_case`, next to
/// validation): one driving site, two harness modes. This binary owns
/// only CLI parsing, round timing, and result printing, so stepping
/// semantics cannot drift between perf and validation.
///
/// Metric note: `steps` counts one stepped frame per step. The measure
/// includes the thread barrier per frame — representative of headless
/// driving, not of raw core throughput.

fn peak_rss_mib() -> Option<f64> {
    #[cfg(target_os = "linux")]
    {
        let status = std::fs::read_to_string("/proc/self/status").ok()?;
        for line in status.lines() {
            if let Some(rest) = line.strip_prefix("VmHWM:") {
                let kib = rest.split_whitespace().next()?.parse::<u64>().ok()?;
                return Some(kib as f64 / 1024.0);
            }
        }
        None
    }
    #[cfg(not(target_os = "linux"))]
    {
        None
    }
}

fn process_cpu_time_nanos() -> Result<u64, String> {
    #[cfg(target_os = "linux")]
    {
        let schedstat = std::fs::read_to_string("/proc/self/schedstat")
            .map_err(|error| format!("failed to read /proc/self/schedstat: {error}"))?;
        schedstat
            .split_whitespace()
            .next()
            .ok_or_else(|| "missing runtime field in /proc/self/schedstat".to_string())?
            .parse::<u64>()
            .map_err(|error| format!("failed to parse CPU time: {error}"))
    }
    #[cfg(not(target_os = "linux"))]
    {
        Err("CPU time measurement is only supported on Linux".to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_sensors_report_on_linux() {
        if cfg!(target_os = "linux") {
            let mib = peak_rss_mib().expect("VmHWM readable");
            assert!(mib > 0.0);
            process_cpu_time_nanos().expect("schedstat readable");
        } else {
            assert_eq!(peak_rss_mib(), None);
            assert!(process_cpu_time_nanos().is_err());
        }
    }

    #[test]
    fn parse_args_defaults_and_overrides() {
        assert_eq!(
            parse_args(["perf"]).expect("defaults"),
            PerfArgs {
                rounds: 5,
                warmup_rounds: 1,
                case_ids: Vec::new(),
            }
        );
        assert_eq!(
            parse_args([
                "perf",
                "--rounds",
                "2",
                "--warmup-rounds",
                "0",
                "--case",
                "a",
                "--case",
                "b"
            ])
            .expect("overrides"),
            PerfArgs {
                rounds: 2,
                warmup_rounds: 0,
                case_ids: vec!["a".to_string(), "b".to_string()],
            }
        );
    }

    #[test]
    fn parse_args_rejects_bad_rounds() {
        assert!(parse_args(["perf", "--rounds", "0"]).is_err());
        assert!(parse_args(["perf", "--rounds", "many"]).is_err());
        assert!(parse_args(["perf", "--warmup-rounds", "many"]).is_err());
        assert!(parse_args(["perf", "--bogus"]).is_err());
    }
}
