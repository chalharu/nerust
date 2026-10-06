//! Single seam to concrete NES construction and session-layer driving.
//!
//! The generic validation harness drives [`TestSystem`], which owns the
//! session execution engine ([`EmuCore`]) built from factory parts. The
//! only NES-concrete name here is the factory itself (plus its test
//! support types): option mapping, space ids, and pad layouts all live
//! in `nes_factory`. The thread does the stepping, inspecting, and audio
//! tapping; this module translates test intent into session calls.

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use nerust_core_traits::{
    CoreConfig,
    audio::{AudioBackend, StereoSample},
    debugger::{InspectRequest, SpaceId, StepUnit},
    factory::CoreFactory,
};
use nerust_gui_shell::emu_core::EmuCore;
use nerust_input_traits::{GuiInput, InputAssignments, InputValue};
use nerust_nes_factory::{Mmc3IrqVariant as FactoryMmc3Variant, NesFactory, TestPadLayout};
use nerust_render_traits::FrameBuffer;

use crate::{error::RomTestError, manifest::Mmc3IrqVariant};

/// A loaded NES system ready for headless driving: session-owned
/// execution plus input writer and nominal-audio tap.
pub struct TestSystem {
    emu: EmuCore,
    gui_input: GuiInput,
    pad_layout: TestPadLayout,
    tap: Arc<Mutex<Vec<StereoSample>>>,
}

/// Build a loaded NES system through `CoreFactory` and wrap it in the
/// session execution engine.
///
/// * Settings come from the factory's own defaults with the video
///   filter pinned to `None` (deterministic palette bytes; loud
///   failure keeps a future settings change from silently altering
///   screenshots).
/// * Both player slots get the P1 profile: the suite drives pad2 and
///   the microphone, which the factory default (P2 unassigned) lacks.
/// * Audio goes to a null backend stamped at the case rate; nominal
///   samples are tapped per stepped frame, never rate-controlled.
/// * Test options (`mmc3_irq_variant`, …) resolve inside the factory
///   (`resolve_load_request`); the adapter only translates its own
///   schema enum to the factory's (entry-point glue).
/// * After paused load the thread is silent until the first step, so
///   stepped frames start from a deterministic power-on frame zero.
pub fn open_nes_system(
    factory: &dyn CoreFactory,
    case_id: &str,
    rom_bytes: &[u8],
    mmc3_irq_variant: Option<Mmc3IrqVariant>,
    audio_sample_rate: u32,
) -> Result<TestSystem, RomTestError> {
    let construction = |message: String| RomTestError::CoreConstruction {
        case_id: case_id.to_string(),
        message,
    };

    let view = factory
        .headless_view()
        .map_err(|error| construction(format!("headless view: {error:?}")))?;

    let base = factory.input_system_factory().default_assignments();
    let assignments = duplicate_p1_to_p2(&base)
        .ok_or_else(|| construction("factory default assigns no P1 profile".to_string()))?;

    let speaker: Box<dyn AudioBackend> = Box::new(NullAudioBackend(audio_sample_rate));
    let parts = factory
        .create_core_and_adapter_with_assignments(&view, speaker, &assignments)
        .map_err(|error| construction(format!("create_core: {error:?}")))?;

    let resolved = factory
        .resolve_load_request(
            &view,
            NesFactory::headless_load_options(mmc3_irq_variant.map(|variant| match variant {
                Mmc3IrqVariant::Sharp => FactoryMmc3Variant::Sharp,
                Mmc3IrqVariant::Nec => FactoryMmc3Variant::Nec,
            })),
        )
        .map_err(|error| construction(format!("resolve options: {error:?}")))?;

    let config = CoreConfig {
        region: None,
        bios_paths: HashMap::new(),
        controllers: HashMap::new(),
        core_options: Some(resolved.options),
        audio_sample_rate: None,
    };
    let (emu, gui_input, field_map, _) = EmuCore::from_parts(parts);
    let pad_layout = NesFactory::test_pad_layout(&field_map)
        .map_err(|error| construction(format!("test pad layout: {error:?}")))?;
    // Paused load: power-on state survives to frame zero (no free-run
    // frames, no compensating reset that would destroy it).
    emu.load_paused(
        &nerust_core_traits::factory::load::MediaObject::new(None, rom_bytes.to_vec()),
        config.core_options,
    )
    .map_err(|error| construction(format!("load: {error:?}")))?;
    let tap = Arc::new(Mutex::new(Vec::new()));
    emu.tap_nominal_audio(Arc::clone(&tap))
        .map_err(|error| construction(format!("tap: {error:?}")))?;

    Ok(TestSystem {
        emu,
        gui_input,
        pad_layout,
        tap,
    })
}

/// Clone the P1 profile onto P2. The suite drives both pads (88 pad2
/// events) plus the microphone, so the factory default of an unassigned
/// P2 is insufficient here.
fn duplicate_p1_to_p2(base: &InputAssignments) -> Option<InputAssignments> {
    let p1 = base.slots.first()?.1.clone()?;
    let mut slots = base.slots.clone();
    if slots.len() > 1 {
        slots[1].1 = Some(p1);
    }
    Some(InputAssignments { slots })
}

impl TestSystem {
    /// Publish absolute pad state for the next frame.
    ///
    /// Bits follow the suite convention (A B Select Start Up Down
    /// Left Right per pad); the factory owns the bit-to-field layout.
    /// Unknown hardware bits are skipped, but a field write failure is
    /// a loud error — never a silent no-op.
    pub fn sync_input(&mut self, pad1: u8, pad2: u8, mic: bool) -> Result<(), RomTestError> {
        let seed = |this: &mut Self, pad: usize, bits: u8| -> Result<(), RomTestError> {
            for bit in 0..8 {
                if let Some(field) = this.pad_layout.pad_fields[pad][bit] {
                    this.gui_input
                        .state
                        .set(field, InputValue::Digital(bits & (1 << bit) != 0))
                        .map_err(|error| RomTestError::EmuThread(format!("seed input: {error}")))?;
                }
            }
            Ok(())
        };
        seed(self, 0, pad1)?;
        seed(self, 1, pad2)?;
        if let Some(field) = self.pad_layout.mic_field {
            self.gui_input
                .state
                .set(field, InputValue::Digital(mic))
                .map_err(|error| RomTestError::EmuThread(format!("seed input: {error}")))?;
        }
        self.gui_input.publish();
        Ok(())
    }

    /// Advance exactly one frame. The reply is the barrier: it arrives
    /// after the frame rendered, tapped, and published.
    pub fn step_frame(&self) -> Result<(), RomTestError> {
        self.emu
            .step(StepUnit::Frame)
            .map_err(|error| RomTestError::EmuThread(format!("transport: {error:?}")))?
            .map_err(|error| RomTestError::EmuThread(format!("step: {error:?}")))?;
        Ok(())
    }

    /// Latest published frame, read from the shared display buffer.
    /// Swaps first: stepped frames land in the shared buffer, and only
    /// the swap moves them to the display side. Valid after a step: the
    /// thread is paused, so no frame can overwrite it concurrently.
    pub fn screen_buffer(&mut self) -> &FrameBuffer {
        self.emu.swap_frame_buffer();
        self.emu.frame_buffer()
    }

    /// Drain the nominal-audio tap (samples from stepped frames only).
    pub fn drain_audio(&self) -> Result<Vec<StereoSample>, RomTestError> {
        self.tap
            .lock()
            .map(|mut guard| std::mem::take(&mut *guard))
            .map_err(|error| RomTestError::EmuThread(format!("tap lock: {error}")))
    }

    /// Read one byte from Work RAM through the thread inspect path.
    ///
    /// The space id resolves by stable table key inside the factory,
    /// so a table reorder can never silently mis-resolve.
    pub fn read_work_ram_byte(&self, addr: u32) -> Result<Option<u8>, RomTestError> {
        self.read_byte(self.space_for_key("wram")?, addr)
    }

    /// Read one byte from PPU VRAM through the thread inspect path.
    pub fn read_ppu_vram_byte(&self, addr: u32) -> Result<Option<u8>, RomTestError> {
        self.read_byte(self.space_for_key("ppu_vram")?, addr)
    }

    fn space_for_key(&self, key: &'static str) -> Result<SpaceId, RomTestError> {
        NesFactory::space_id_for_key(key)
            .ok_or_else(|| RomTestError::EmuThread(format!("unknown memory space key: {key}")))
    }

    fn read_byte(&self, space: SpaceId, addr: u32) -> Result<Option<u8>, RomTestError> {
        let inner = self
            .emu
            .inspect(InspectRequest {
                space: Some(space),
                addr: Some(addr),
                rows: 1,
            })
            .map_err(|error| RomTestError::EmuThread(format!("transport: {error:?}")))?
            .map_err(|error| RomTestError::EmuThread(format!("inspect: {error:?}")))?;
        Ok(inner
            .dump
            .rows
            .first()
            .filter(|row| row.valid > 0)
            .map(|row| row.bytes[0]))
    }

    /// Reset emulation to a deterministic frame zero.
    pub fn reset(&self) -> Result<(), RomTestError> {
        self.emu
            .reset()
            .map_err(|error| RomTestError::EmuThread(format!("reset: {error:?}")))
    }

    /// Cartridge-RAM peek preserving bus state (mapped vs open bus).
    pub fn peek_cartridge_ram(&self, address: usize) -> Result<Option<(u8, bool)>, RomTestError> {
        self.emu
            .peek_cartridge_ram(address)
            .map_err(|error| RomTestError::EmuThread(format!("transport: {error:?}")))
    }
}

/// Silent audio backend stamped at the case rate. The thread stamps the
/// backend rate into the console resampler, so the null backend carries
/// the case rate and nominal samples stay comparable to expectations.
struct NullAudioBackend(u32);

impl AudioBackend for NullAudioBackend {
    fn start(&mut self) {}
    fn pause(&mut self) {}
    fn push(&mut self, _sample: StereoSample) {}
    fn sample_rate(&self) -> u32 {
        self.0
    }
}
