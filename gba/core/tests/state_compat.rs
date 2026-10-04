//! Forward-compatibility: a state blob written at the Phase 12 start
//! (before the `pending_oam` / `intrwait_mask` wire fields existed) must
//! still load. The fixture was produced by `save_state` on the
//! Phase-12-base commit with `nba_dma_start_delay` after two frames;
//! the new fields fall back to live-OAM seeding and an unrestricted
//! wake mask respectively.
use nerust_core_traits::{ConsoleCore, CoreConfig};
use nerust_gba_core::console_core::GbaConsoleCore;
use nerust_gba_core::input_types::GbaInputBuffer;
use nerust_input_traits::{EmuInput, InputStateBuffer};
use std::collections::HashMap;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

fn test_core() -> GbaConsoleCore {
    let shared: Arc<Mutex<Box<dyn InputStateBuffer>>> =
        Arc::new(Mutex::new(Box::<GbaInputBuffer>::default()));
    GbaConsoleCore::new(EmuInput::new(
        shared,
        Arc::new(AtomicBool::new(false)),
        Box::new(|| Box::<GbaInputBuffer>::default()),
    ))
}

fn test_config() -> CoreConfig {
    CoreConfig {
        region: None,
        bios_paths: HashMap::new(),
        controllers: HashMap::new(),
        core_options: None,
        audio_sample_rate: None,
    }
}

#[test]
fn phase12_start_state_loads_and_runs() {
    let mut core = test_core();
    let rom =
        std::fs::read("../../roms/gba/nba-emu_hw-test/dma/start-delay/start-delay.gba").unwrap();
    core.load(&rom, &test_config()).unwrap();
    let blob = std::fs::read("tests/fixtures/phase12-start-state.bin").unwrap();
    core.load_state(&blob).unwrap();
    // The imported state (with defaulted new fields) must be fully
    // usable: render a frame, re-export, and load into a fresh core.
    let mut frame = nerust_render_traits::FrameBuffer::with_capacity(
        240,
        160,
        nerust_render_traits::PixelFormat::Rgba,
    );
    core.render_frame(&mut frame, &mut Vec::new()).unwrap();
    let again = core.save_state().unwrap();
    let mut fresh = test_core();
    fresh.load(&rom, &test_config()).unwrap();
    fresh.load_state(&again).unwrap();
    fresh.render_frame(&mut frame, &mut Vec::new()).unwrap();
}
