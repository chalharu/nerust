use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use clap::{Arg, ArgAction, Command};
use nerust_core_traits::audio::AudioBackend;
use nerust_input_traits::{ControllerCollection, ControllerHub as _};
use nerust_nes_core::{Core, rom_parse};
use nerust_nes_device::famicom_set::{FamicomPadP1, FamicomPadP2};
use nerust_render_base::{FrameBuffer, PixelFormat, filter::FilterType};

use crate::{
    error::RomTestError,
    events::{ButtonCode, Buttons, ControllerPad, PadState, RomAssertion},
    harness::{CaseHarness, apply_button_state, drive_case},
    manifest::{RomCase, load_default_manifest, read_rom},
    results::{CaseOutcome, ValidationOptions},
    runner::validate_case,
};

pub fn run_cli() {
    if let Err(message) = run() {
        eprintln!("{message}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
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
        .get_matches();

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
    for case in &cases {
        match validate_case(
            case,
            ValidationOptions {
                capture_screenshots: false,
                check_expectations: true,
            },
        ) {
            CaseOutcome::Completed(validation) if validation.passed() => {
                println!(
                    "validated case={} frames={} steps={} final_hash=0x{:016X} audio_samples={} audio_hash=0x{:016X}",
                    validation.case_id,
                    validation.frames,
                    validation.steps,
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
        }

        roms.push(
            read_rom(case)
                .map_err(|error| error.to_string())
                .map(|bytes| (*case, bytes))?,
        );
    }

    for _ in 0..warmup_rounds {
        for (case, rom_bytes) in &roms {
            let result = PerfRunner::new(case, rom_bytes)
                .map_err(|error| error.to_string())?
                .run(case)
                .map_err(|error| error.to_string())?;
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
            let result = PerfRunner::new(case, rom_bytes)
                .map_err(|error| error.to_string())?
                .run(case)
                .map_err(|error| error.to_string())?;
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

struct PerfRunner {
    core: Core,
    screen: FrameBuffer,
    checksum: u64,
    controller: ControllerCollection,
    mixer: PerfMixer,
    frame_counter: u64,
    pad1: Buttons,
    pad2: Buttons,
    mic: bool,
}

impl PerfRunner {
    fn new(case: &RomCase, rom_bytes: &[u8]) -> Result<Self, RomTestError> {
        let cartridge_data =
            rom_parse::parse_rom(rom_bytes).map_err(|error| RomTestError::CoreConstruction {
                case_id: case.id.clone(),
                message: error.to_string(),
            })?;
        let core =
            Core::new_with_options(cartridge_data, case.core_options()).map_err(|error| {
                RomTestError::CoreConstruction {
                    case_id: case.id.clone(),
                    message: error.to_string(),
                }
            })?;
        let mut palette = [0u32; 256];
        let assets = FilterType::NtscComposite.palette_console_video_assets();
        let rgba8 = assets.palette_rgba8();
        for (i, entry) in palette.iter_mut().enumerate().take(64) {
            let pos = i * 4;
            *entry = u32::from(rgba8[pos]) << 24
                | u32::from(rgba8[pos + 1]) << 16
                | u32::from(rgba8[pos + 2]) << 8
                | u32::from(rgba8[pos + 3]);
        }
        let mut screen = FrameBuffer::with_capacity(
            256,
            240,
            PixelFormat::PaletteIndex {
                palette: Box::new(palette),
            },
        );
        screen.resize(256, 240);
        Ok(Self {
            core,
            screen,
            checksum: 0,
            controller: ControllerCollection::new(vec![
                Box::new(FamicomPadP1::new()),
                Box::new(FamicomPadP2::new()),
            ]),
            mixer: PerfMixer::new(case.audio_sample_rate()),
            frame_counter: 0,
            pad1: Buttons::empty(),
            pad2: Buttons::empty(),
            mic: false,
        })
    }

    fn run(mut self, case: &RomCase) -> Result<PerfRunResult, RomTestError> {
        let totals = drive_case(case, &mut self)?;
        Ok(PerfRunResult {
            frames: totals.frames,
            steps: totals.steps,
            final_marker: self.checksum,
        })
    }
}

impl CaseHarness for PerfRunner {
    fn run_frame(&mut self) -> u64 {
        let steps = self
            .core
            .run_frame(&mut self.screen, &mut self.controller, &mut self.mixer);
        // Per-frame checksum: PPU が FrameBuffer に書き込んだ全ピクセルから計算
        for &b in self.screen.as_ref() {
            self.checksum = self.checksum.wrapping_mul(31).wrapping_add(u64::from(b));
        }
        self.frame_counter += 1;
        steps
    }

    fn frame_counter(&self) -> u64 {
        self.frame_counter
    }

    fn on_assert(&mut self, _frame: u64, _assertion: &RomAssertion) -> Result<(), RomTestError> {
        Ok(())
    }

    fn on_reset(&mut self) -> Result<(), RomTestError> {
        self.core.reset();
        Ok(())
    }

    fn on_standard_controller(
        &mut self,
        pad: ControllerPad,
        button: ButtonCode,
        state: PadState,
    ) -> Result<(), RomTestError> {
        let buttons = Buttons::from(button);
        match pad {
            ControllerPad::Pad1 => {
                self.pad1 = apply_button_state(self.pad1, buttons, state);
            }
            ControllerPad::Pad2 => {
                self.pad2 = apply_button_state(self.pad2, buttons, state);
            }
        }
        self.controller
            .sync_input(&[self.pad1.bits(), self.pad2.bits(), self.mic as u8]);
        Ok(())
    }

    fn on_microphone(&mut self, state: PadState) -> Result<(), RomTestError> {
        self.mic = matches!(state, PadState::Pressed);
        self.controller
            .sync_input(&[self.pad1.bits(), self.pad2.bits(), self.mic as u8]);
        Ok(())
    }
}

#[derive(Debug, Clone, Copy)]
struct PerfRunResult {
    frames: u64,
    steps: u64,
    final_marker: u64,
}

struct PerfMixer {
    sample_rate: u32,
}

impl PerfMixer {
    fn new(sample_rate: u32) -> Self {
        Self { sample_rate }
    }
}

impl AudioBackend for PerfMixer {
    fn start(&mut self) {}
    fn pause(&mut self) {}
    fn push(&mut self, _data: f32) {}

    fn sample_rate(&self) -> u32 {
        self.sample_rate
    }
}

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
