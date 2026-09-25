//! TEMPORARY bit-exact A/B harness for perf refactors. DELETE BEFORE MERGE.
//!
//! Captures per-frame framebuffer CRC64 + cumulative audio sample hash so
//! that behavior-preserving optimizations can be validated bit-exactly.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::{Arc, Mutex, atomic::AtomicBool};

use nerust_core_traits::{
    ConsoleCore, CoreConfig,
    audio::{AudioBackend, StereoSample},
};
use nerust_gba_core::console_core::GbaConsoleCore;
use nerust_gba_core::input_types::GbaInputBuffer;
use nerust_input_traits::{EmuInput, InputStateBuffer};
use nerust_render_traits::{FrameBuffer, PixelFormat};

#[derive(Debug, Default)]
struct TapAudio {
    samples: Arc<Mutex<Vec<(u32, u32)>>>,
}

impl AudioBackend for TapAudio {
    fn start(&mut self) {}
    fn pause(&mut self) {}
    fn sample_rate(&self) -> u32 {
        32768
    }
    fn push(&mut self, sample: StereoSample) {
        self.samples.lock().unwrap().push((
            sample.left.to_bits(),
            sample.right.to_bits(),
        ));
    }
}

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
    let tap = TapAudio::default();
    let samples = Arc::clone(&tap.samples);
    let mut core = GbaConsoleCore::new(Box::new(tap), emu_input());
    core.load(&rom, &config).unwrap();
    let mut slot = FrameBuffer::with_capacity(240, 160, PixelFormat::Rgba);
    let mut fb_hasher = DefaultHasher::new();
    for _ in 0..frames {
        core.render_frame(&mut slot).unwrap();
        slot.as_ref().hash(&mut fb_hasher);
    }
    let mut audio_hasher = DefaultHasher::new();
    samples.lock().unwrap().as_slice().hash(&mut audio_hasher);
    println!(
        "{name}: fb_crc={:016x} audio_crc={:016x} nsamples={}",
        fb_hasher.finish(),
        audio_hasher.finish(),
        samples.lock().unwrap().len()
    );
}

#[test]
fn tmp_ab_golden() {
    run_case("suite", "../../roms/gba/mgba-suite/suite.gba", 120);
    run_case("hello", "../../roms/gba/jsmolka_gba-tests/ppu/hello.gba", 120);
}
