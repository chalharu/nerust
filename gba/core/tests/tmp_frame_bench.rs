//! TEMPORARY frame-time benchmark for perf investigation. DELETE BEFORE MERGE.

use std::sync::{Arc, Mutex, atomic::AtomicBool};
use std::time::Instant;

use nerust_core_traits::{ConsoleCore, CoreConfig, audio::NullAudio};
use nerust_gba_core::console_core::GbaConsoleCore;
use nerust_gba_core::input_types::GbaInputBuffer;
use nerust_input_traits::{EmuInput, InputStateBuffer};
use nerust_render_traits::{FrameBuffer, PixelFormat};

fn emu_input() -> EmuInput {
    let shared: Arc<Mutex<Box<dyn InputStateBuffer>>> =
        Arc::new(Mutex::new(Box::<GbaInputBuffer>::default()));
    EmuInput::new(
        shared,
        Arc::new(AtomicBool::new(false)),
        Box::new(|| Box::<GbaInputBuffer>::default()),
    )
}

fn run_case(name: &str, path: &str, frames: u32) {
    let rom = std::fs::read(path).unwrap();
    let config = CoreConfig {
        region: None,
        bios_paths: Default::default(),
        controllers: Default::default(),
        core_options: None,
    };
    let mut core = GbaConsoleCore::new(Box::new(NullAudio), emu_input());
    core.load(&rom, &config).unwrap();
    let mut slot = FrameBuffer::with_capacity(240, 160, PixelFormat::Rgba);
    for _ in 0..5 {
        core.render_frame(&mut slot).unwrap();
    }
    let start = Instant::now();
    for _ in 0..frames {
        core.render_frame(&mut slot).unwrap();
    }
    let ms = start.elapsed().as_secs_f64() * 1000.0 / f64::from(frames);
    println!("{name}: {ms:.2} ms/frame ({:.1} fps)", 1000.0 / ms);
}

#[test]
fn tmp_frame_bench() {
    let only = std::env::var("TMP_BENCH_ONLY").unwrap_or_default();
    if only.is_empty() || only == "suite" {
        run_case("suite", "../../roms/gba/mgba-suite/suite.gba", 120);
    }
    if only.is_empty() || only == "hello" {
        run_case("hello", "../../roms/gba/jsmolka_gba-tests/ppu/hello.gba", 120);
    }
}
