use crc::{CRC_64_XZ, Crc, Digest};
use nerust_core::Core;
use nerust_core::controller::standard_controller::{Buttons, StandardController};
use nerust_screen_buffer::ScreenBuffer;
use nerust_screen_filter::FilterType;
use nerust_screen_traits::LogicalSize;
use nerust_sound_traits::MixerInput;
use std::env;
use std::fs;
use std::hash::{Hash, Hasher};
use std::time::{Duration, Instant};

const CRC64_LEGACY_ECMA: Crc<u64> = Crc::<u64>::new(&CRC_64_XZ);
const DEFAULT_ROUNDS: usize = 5;
const DEFAULT_WARMUP_ROUNDS: usize = 1;
const PERF_MIXER_SAMPLE_RATE: u32 = 192_000;

struct Crc64Hasher(Digest<'static, u64>);

impl Crc64Hasher {
    fn new() -> Self {
        Self(CRC64_LEGACY_ECMA.digest())
    }
}

impl Hasher for Crc64Hasher {
    fn write(&mut self, bytes: &[u8]) {
        self.0.update(bytes);
    }

    fn finish(&self) -> u64 {
        self.0.clone().finalize()
    }
}

#[derive(Debug, Clone)]
struct TestMixer {
    samples: u64,
    checksum: u64,
}

impl TestMixer {
    const FNV_OFFSET_BASIS: u64 = 0xCBF2_9CE4_8422_2325;
    const FNV_PRIME: u64 = 0x0000_0100_0000_01B3;

    fn new() -> Self {
        Self {
            samples: 0,
            checksum: Self::FNV_OFFSET_BASIS,
        }
    }

    fn samples(&self) -> u64 {
        self.samples
    }

    fn checksum(&self) -> u64 {
        self.checksum
    }
}

impl MixerInput for TestMixer {
    fn push(&mut self, data: f32) {
        self.samples += 1;
        self.checksum ^= u64::from(data.to_bits());
        self.checksum = self.checksum.wrapping_mul(Self::FNV_PRIME);
    }

    fn sample_rate(&self) -> u32 {
        PERF_MIXER_SAMPLE_RATE
    }
}

struct PerfMixer;

impl MixerInput for PerfMixer {
    fn push(&mut self, _data: f32) {}

    fn sample_rate(&self) -> u32 {
        PERF_MIXER_SAMPLE_RATE
    }
}

struct PerfScreen {
    checksum: u64,
    frame_pixels: u32,
}

impl PerfScreen {
    const FNV_OFFSET_BASIS: u64 = 0xCBF2_9CE4_8422_2325;
    const FNV_PRIME: u64 = 0x0000_0100_0000_01B3;

    fn new() -> Self {
        Self {
            checksum: Self::FNV_OFFSET_BASIS,
            frame_pixels: 0,
        }
    }

    fn checksum(&self) -> u64 {
        self.checksum
    }
}

impl nerust_screen_traits::Screen for PerfScreen {
    fn push(&mut self, value: u8) {
        self.checksum ^= u64::from(value);
        self.checksum = self.checksum.wrapping_mul(Self::FNV_PRIME);
        self.frame_pixels += 1;
    }

    fn render(&mut self) {
        self.checksum ^= u64::from(self.frame_pixels);
        self.checksum = self.checksum.wrapping_mul(Self::FNV_PRIME);
        self.frame_pixels = 0;
    }
}

#[derive(Debug, Clone, Copy)]
enum ButtonCode {
    Select,
    Start,
}

impl From<ButtonCode> for Buttons {
    fn from(value: ButtonCode) -> Self {
        match value {
            ButtonCode::Select => Buttons::SELECT,
            ButtonCode::Start => Buttons::START,
        }
    }
}

#[derive(Debug, Clone, Copy)]
enum PadState {
    Pressed,
    Released,
}

#[derive(Debug, Clone, Copy)]
enum EventKind {
    CheckScreen(u64),
    Pad1(ButtonCode, PadState),
}

#[derive(Debug, Clone, Copy)]
struct Event {
    frame_number: u64,
    kind: EventKind,
}

impl Event {
    const fn check_screen(frame_number: u64, hash: u64) -> Self {
        Self {
            frame_number,
            kind: EventKind::CheckScreen(hash),
        }
    }

    const fn pad1(frame_number: u64, button: ButtonCode, state: PadState) -> Self {
        Self {
            frame_number,
            kind: EventKind::Pad1(button, state),
        }
    }
}

struct RomCase {
    name: &'static str,
    rom: &'static [u8],
    events: &'static [Event],
    expected_audio_samples: u64,
    expected_audio_hash: u64,
}

impl RomCase {
    const fn new(
        name: &'static str,
        rom: &'static [u8],
        events: &'static [Event],
        expected_audio_samples: u64,
        expected_audio_hash: u64,
    ) -> Self {
        Self {
            name,
            rom,
            events,
            expected_audio_samples,
            expected_audio_hash,
        }
    }

    fn final_frame(&self) -> u64 {
        self.events
            .last()
            .map(|event| event.frame_number)
            .expect("perf cases must have at least one event")
    }
}

const NESTEST_ROM: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../roms/cpu/nestest.nes"
));
const APU_LEN_CTR_ROM: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../roms/apu/blargg_apu_2005.07.30/01.len_ctr.nes"
));
const PPU_VBL_NMI_ROM: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../roms/ppu/ppu_vbl_nmi/ppu_vbl_nmi.nes"
));

const NESTEST_EVENTS: &[Event] = &[
    Event::check_screen(15, 0x4640_33EF_DAB1_1D8E),
    Event::pad1(15, ButtonCode::Start, PadState::Pressed),
    Event::pad1(16, ButtonCode::Start, PadState::Released),
    Event::check_screen(70, 0xBE54_DF8C_F9FB_E026),
    Event::pad1(70, ButtonCode::Select, PadState::Pressed),
    Event::pad1(71, ButtonCode::Select, PadState::Released),
    Event::check_screen(75, 0x9D08_2986_B6F8_DF51),
    Event::pad1(75, ButtonCode::Start, PadState::Pressed),
    Event::pad1(76, ButtonCode::Start, PadState::Released),
    Event::check_screen(90, 0xBACF_3F4F_CBF5_718C),
];

const APU_LEN_CTR_EVENTS: &[Event] = &[Event::check_screen(30, 0xE31E_B517_2247_2E30)];
const PPU_VBL_NMI_EVENTS: &[Event] = &[Event::check_screen(1640, 0xEB57_E169_78E4_5540)];

const CASES: &[RomCase] = &[
    RomCase::new(
        "cpu.nestest",
        NESTEST_ROM,
        NESTEST_EVENTS,
        287_270,
        0x34BB_3FFD_F962_043D,
    ),
    RomCase::new(
        "apu.len_ctr",
        APU_LEN_CTR_ROM,
        APU_LEN_CTR_EVENTS,
        95_586,
        0x27C6_A0AD_8041_E1F7,
    ),
    RomCase::new(
        "ppu.vbl_nmi",
        PPU_VBL_NMI_ROM,
        PPU_VBL_NMI_EVENTS,
        5_239_142,
        0x2E61_2CB2_A8E5_80BD,
    ),
];

struct ScenarioRunner {
    screen_buffer: ScreenBuffer,
    core: Core,
    controller: StandardController,
    mixer: TestMixer,
    frame_counter: u64,
    pad1: Buttons,
}

impl ScenarioRunner {
    fn new(rom: &'static [u8]) -> Self {
        let mut iter = rom.iter().copied();
        Self {
            screen_buffer: ScreenBuffer::new(
                FilterType::None,
                LogicalSize {
                    width: 256,
                    height: 240,
                },
            ),
            core: Core::new(&mut iter).expect("failed to construct core"),
            controller: StandardController::new(),
            mixer: TestMixer::new(),
            frame_counter: 0,
            pad1: Buttons::empty(),
        }
    }

    fn run_case(mut self, case: &RomCase) -> ValidationResult {
        let mut total_steps = 0_u64;
        let mut next_event = 0_usize;

        while self.frame_counter < case.final_frame() {
            total_steps += self.run_frame();
            while let Some(event) = case.events.get(next_event) {
                if event.frame_number != self.frame_counter {
                    break;
                }
                self.apply_event(*event);
                next_event += 1;
            }
        }

        let audio_samples = self.mixer.samples();
        let audio_hash = self.mixer.checksum();
        assert_eq!(
            audio_samples, case.expected_audio_samples,
            "audio sample count mismatch for {}",
            case.name
        );
        assert_eq!(
            audio_hash, case.expected_audio_hash,
            "audio hash mismatch for {}",
            case.name
        );

        ValidationResult {
            frames: self.frame_counter,
            steps: total_steps,
            final_hash: self.screen_hash(),
            audio_samples,
            audio_hash,
        }
    }

    fn run_frame(&mut self) -> u64 {
        let steps = self.core.run_frame(
            &mut self.screen_buffer,
            &mut self.controller,
            &mut self.mixer,
        );
        self.frame_counter += 1;
        steps
    }

    fn apply_event(&mut self, event: Event) {
        match event.kind {
            EventKind::CheckScreen(expected_hash) => {
                let actual_hash = self.screen_hash();
                assert_eq!(
                    actual_hash, expected_hash,
                    "screen hash mismatch for frame {}",
                    self.frame_counter
                );
            }
            EventKind::Pad1(button, state) => self.apply_pad1(button, state),
        }
    }

    fn screen_hash(&self) -> u64 {
        let mut hasher = Crc64Hasher::new();
        self.screen_buffer.hash(&mut hasher);
        hasher.finish()
    }

    fn apply_pad1(&mut self, button: ButtonCode, state: PadState) {
        self.pad1 = match state {
            PadState::Pressed => self.pad1 | Buttons::from(button),
            PadState::Released => self.pad1 & !Buttons::from(button),
        };
        self.controller.set_pad1(self.pad1);
    }
}

struct PerfRunner {
    screen: PerfScreen,
    core: Core,
    controller: StandardController,
    mixer: PerfMixer,
    frame_counter: u64,
    pad1: Buttons,
}

impl PerfRunner {
    fn new(rom: &'static [u8]) -> Self {
        let mut iter = rom.iter().copied();
        Self {
            screen: PerfScreen::new(),
            core: Core::new(&mut iter).expect("failed to construct core"),
            controller: StandardController::new(),
            mixer: PerfMixer,
            frame_counter: 0,
            pad1: Buttons::empty(),
        }
    }

    fn run_case(mut self, case: &RomCase) -> Result<RunResult, String> {
        let wall_started = Instant::now();
        let cpu_started_nanos = process_cpu_time_nanos()?;
        let mut total_steps = 0_u64;
        let mut next_event = 0_usize;

        while self.frame_counter < case.final_frame() {
            total_steps += self.run_frame();
            while let Some(event) = case.events.get(next_event) {
                if event.frame_number != self.frame_counter {
                    break;
                }
                if let EventKind::Pad1(button, state) = event.kind {
                    self.apply_pad1(button, state);
                }
                next_event += 1;
            }
        }

        Ok(RunResult {
            frames: self.frame_counter,
            steps: total_steps,
            wall_duration: wall_started.elapsed(),
            cpu_duration: Duration::from_nanos(
                process_cpu_time_nanos()?.saturating_sub(cpu_started_nanos),
            ),
            final_hash: self.screen.checksum(),
        })
    }

    fn run_frame(&mut self) -> u64 {
        let steps = self
            .core
            .run_frame(&mut self.screen, &mut self.controller, &mut self.mixer);
        self.frame_counter += 1;
        steps
    }

    fn apply_pad1(&mut self, button: ButtonCode, state: PadState) {
        self.pad1 = match state {
            PadState::Pressed => self.pad1 | Buttons::from(button),
            PadState::Released => self.pad1 & !Buttons::from(button),
        };
        self.controller.set_pad1(self.pad1);
    }
}

#[derive(Debug, Clone, Copy)]
struct RunResult {
    frames: u64,
    steps: u64,
    wall_duration: Duration,
    cpu_duration: Duration,
    final_hash: u64,
}

#[derive(Debug, Clone, Copy)]
struct ValidationResult {
    frames: u64,
    steps: u64,
    final_hash: u64,
    audio_samples: u64,
    audio_hash: u64,
}

#[derive(Debug, Clone)]
struct Config {
    rounds: usize,
    warmup_rounds: usize,
    case_filter: Option<String>,
}

impl Config {
    fn parse() -> Result<Self, String> {
        let mut rounds = DEFAULT_ROUNDS;
        let mut warmup_rounds = DEFAULT_WARMUP_ROUNDS;
        let mut case_filter = None;
        let mut args = env::args().skip(1);

        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--rounds" => {
                    let value = args.next().ok_or("--rounds requires a value")?;
                    rounds = value
                        .parse::<usize>()
                        .map_err(|_| format!("invalid --rounds value: {value}"))?;
                }
                "--warmup-rounds" => {
                    let value = args.next().ok_or("--warmup-rounds requires a value")?;
                    warmup_rounds = value
                        .parse::<usize>()
                        .map_err(|_| format!("invalid --warmup-rounds value: {value}"))?;
                }
                "--case" => {
                    case_filter = Some(args.next().ok_or("--case requires a value")?);
                }
                "--help" | "-h" => {
                    print_help();
                    std::process::exit(0);
                }
                _ => return Err(format!("unknown argument: {arg}")),
            }
        }

        if rounds == 0 {
            return Err("--rounds must be greater than 0".to_string());
        }

        Ok(Self {
            rounds,
            warmup_rounds,
            case_filter,
        })
    }
}

#[derive(Default)]
struct Aggregate {
    total_wall_duration_secs: f64,
    total_cpu_duration_secs: f64,
    total_steps: u64,
    total_frames: u64,
}

fn main() {
    if let Err(message) = run() {
        eprintln!("{message}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let config = Config::parse()?;
    let cases = selected_cases(config.case_filter.as_deref())?;

    println!(
        "perf-suite rounds={} warmup_rounds={} cases={}",
        config.rounds,
        config.warmup_rounds,
        cases.len()
    );

    for _ in 0..config.warmup_rounds {
        for case in &cases {
            let result = PerfRunner::new(case.rom).run_case(case)?;
            std::hint::black_box(result.final_hash);
        }
    }

    let mut suite = Aggregate::default();
    for case in &cases {
        let mut aggregate = Aggregate::default();
        let mut final_hash = 0_u64;

        for round in 0..config.rounds {
            let result = PerfRunner::new(case.rom).run_case(case)?;
            final_hash = result.final_hash;
            aggregate.total_wall_duration_secs += result.wall_duration.as_secs_f64();
            aggregate.total_cpu_duration_secs += result.cpu_duration.as_secs_f64();
            aggregate.total_steps += result.steps;
            aggregate.total_frames += result.frames;

            println!(
                "run round={} case={} cpu_time_ms={:.3} wall_time_ms={:.3} frames={} steps={} steps_per_cpu_sec={:.3} steps_per_wall_sec={:.3}",
                round + 1,
                case.name,
                result.cpu_duration.as_secs_f64() * 1_000.0,
                result.wall_duration.as_secs_f64() * 1_000.0,
                result.frames,
                result.steps,
                result.steps as f64 / result.cpu_duration.as_secs_f64(),
                result.steps as f64 / result.wall_duration.as_secs_f64(),
            );
        }

        suite.total_wall_duration_secs += aggregate.total_wall_duration_secs;
        suite.total_cpu_duration_secs += aggregate.total_cpu_duration_secs;
        suite.total_steps += aggregate.total_steps;
        suite.total_frames += aggregate.total_frames;

        let avg_wall_duration_secs = aggregate.total_wall_duration_secs / config.rounds as f64;
        let avg_cpu_duration_secs = aggregate.total_cpu_duration_secs / config.rounds as f64;
        let avg_steps = aggregate.total_steps as f64 / config.rounds as f64;
        let avg_frames = aggregate.total_frames as f64 / config.rounds as f64;

        println!(
            "summary case={} avg_cpu_time_ms={:.3} avg_wall_time_ms={:.3} avg_frames={:.1} avg_steps={:.1} avg_steps_per_cpu_sec={:.3} avg_steps_per_wall_sec={:.3} avg_frames_per_cpu_sec={:.3} final_marker=0x{final_hash:016X}",
            case.name,
            avg_cpu_duration_secs * 1_000.0,
            avg_wall_duration_secs * 1_000.0,
            avg_frames,
            avg_steps,
            avg_steps / avg_cpu_duration_secs,
            avg_steps / avg_wall_duration_secs,
            avg_frames / avg_cpu_duration_secs,
        );
    }

    let suite_avg_wall_duration_secs = suite.total_wall_duration_secs / config.rounds as f64;
    let suite_avg_cpu_duration_secs = suite.total_cpu_duration_secs / config.rounds as f64;
    let suite_avg_steps = suite.total_steps as f64 / config.rounds as f64;
    let suite_avg_frames = suite.total_frames as f64 / config.rounds as f64;
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

    for case in &cases {
        let result = ScenarioRunner::new(case.rom).run_case(case);
        println!(
            "validated case={} frames={} steps={} final_hash=0x{:016X} audio_samples={} audio_hash=0x{:016X}",
            case.name,
            result.frames,
            result.steps,
            result.final_hash,
            result.audio_samples,
            result.audio_hash
        );
        std::hint::black_box(result.final_hash);
        std::hint::black_box(result.audio_hash);
    }

    Ok(())
}

fn selected_cases(case_filter: Option<&str>) -> Result<Vec<&'static RomCase>, String> {
    let cases = CASES
        .iter()
        .filter(|case| case_filter.is_none_or(|filter| case.name == filter))
        .collect::<Vec<_>>();
    if cases.is_empty() {
        return Err(format!(
            "no perf cases matched {}",
            case_filter.unwrap_or("<all cases>")
        ));
    }
    Ok(cases)
}

fn peak_rss_mib() -> Option<f64> {
    #[cfg(target_os = "linux")]
    {
        let status = fs::read_to_string("/proc/self/status").ok()?;
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
        let schedstat = fs::read_to_string("/proc/self/schedstat")
            .map_err(|error| format!("failed to read /proc/self/schedstat: {error}"))?;
        let nanos = schedstat
            .split_whitespace()
            .next()
            .ok_or("missing runtime field in /proc/self/schedstat")?
            .parse::<u64>()
            .map_err(|error| format!("failed to parse CPU time: {error}"))?;
        Ok(nanos)
    }
    #[cfg(not(target_os = "linux"))]
    {
        Err("CPU time measurement is only supported on Linux".to_string())
    }
}

fn print_help() {
    println!(
        "Usage: cargo run -p nerust_core --example perf --release -- [--rounds N] [--warmup-rounds N] [--case NAME]"
    );
    println!("CPU-time measurement requires Linux /proc/self/schedstat.");
}
