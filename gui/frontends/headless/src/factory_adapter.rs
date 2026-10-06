//! Single seam to system construction and session-layer driving.
//!
//! The generic validation harness drives [`TestSystem`], which owns the
//! session execution engine ([`EmuCore`]) built from factory parts. This
//! module names no system-concrete types: [`open_nes_system`] takes
//! `&dyn CoreFactory`, and all system knowledge below is harness-owned
//! *data* (the `TEST_*` tables) resolved through generic trait calls.
//! Production code carries no test support.

use std::{
    borrow::Cow,
    collections::HashMap,
    sync::{Arc, Mutex},
};

use nerust_core_traits::{
    CoreConfig,
    audio::{AudioBackend, StereoSample},
    debugger::{InspectRequest, SpaceId, StepUnit},
    factory::{
        CoreFactory,
        descriptor::{SystemSettingsChoiceId, SystemSettingsFieldId},
    },
};
use nerust_gui_shell::emu_core::EmuCore;
use nerust_input_traits::{AttachmentId, DigitalControlId, GuiInput, InputAssignments, InputValue};
use nerust_render_traits::FrameBuffer;

use crate::{error::RomTestError, manifest::Mmc3IrqVariant};

/// NES headless test profile. System knowledge owned by the harness as
/// data: stable ids from the factory's settings/input descriptors.
/// Any drift fails loudly at open; nothing here can silently
/// mis-resolve.
const TEST_MMC3_FIELD: &str = "core.mmc3_irq_variant";
const TEST_MMC3_SHARP: &str = "sharp";
const TEST_MMC3_NEC: &str = "nec";
const TEST_SPACE_WRAM_KEY: &str = "wram";
const TEST_SPACE_PPU_VRAM_KEY: &str = "ppu_vram";
const TEST_ATTACH_P1: &str = "nes.attachment.player1";
const TEST_ATTACH_P2: &str = "nes.attachment.player2";
const TEST_MIC_CONTROL: &str = "famicom.microphone";
/// Suite bit order per pad: A B Select Start Up Down Left Right.
const TEST_PAD_BUTTONS: [&str; 8] = [
    "nes.control.a",
    "nes.control.b",
    "nes.control.select",
    "nes.control.start",
    "nes.control.up",
    "nes.control.down",
    "nes.control.left",
    "nes.control.right",
];

/// A loaded NES system ready for headless driving: session-owned
/// execution plus input writer and nominal-audio tap.
pub struct TestSystem {
    emu: EmuCore,
    gui_input: GuiInput,
    pad_fields: [[Option<usize>; 8]; 2],
    mic_field: Option<usize>,
    work_ram: SpaceId,
    ppu_vram: SpaceId,
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
/// * Test options (`mmc3_irq_variant`, …) apply through the generic
///   settings-choice port; the adapter only translates its own schema
///   enum to choice strings.
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

    let mut view = factory
        .headless_view()
        .map_err(|error| construction(format!("headless view: {error:?}")))?;
    if let Some(choice) = mmc3_irq_variant.map(|variant| match variant {
        Mmc3IrqVariant::Sharp => TEST_MMC3_SHARP,
        Mmc3IrqVariant::Nec => TEST_MMC3_NEC,
    }) {
        factory
            .apply_settings_choice(
                &mut view,
                &SystemSettingsFieldId(Cow::Borrowed(TEST_MMC3_FIELD)),
                &SystemSettingsChoiceId(Cow::Borrowed(choice)),
            )
            .map_err(|error| construction(format!("apply test option: {error:?}")))?;
    }
    let resolved = factory
        .resolve_load_request(&view, factory.default_load_options())
        .map_err(|error| construction(format!("resolve options: {error:?}")))?;

    let base = factory.input_system_factory().default_assignments();
    let assignments = duplicate_p1_to_p2(&base)
        .ok_or_else(|| construction("factory default assigns no P1 profile".to_string()))?;

    let speaker: Box<dyn AudioBackend> = Box::new(NullAudioBackend(audio_sample_rate));
    let parts = factory
        .create_core_and_adapter_with_assignments(&view, speaker, &assignments)
        .map_err(|error| construction(format!("create_core: {error:?}")))?;

    let config = CoreConfig {
        region: None,
        bios_paths: HashMap::new(),
        controllers: HashMap::new(),
        core_options: Some(resolved.options),
        audio_sample_rate: None,
    };
    let (emu, gui_input, field_map, _) = EmuCore::from_parts(parts);
    // Paused load: power-on state survives to frame zero (no free-run
    // frames, no compensating reset that would destroy it).
    emu.load_paused(
        &nerust_core_traits::factory::load::MediaObject::new(None, rom_bytes.to_vec()),
        config.core_options,
    )
    .map_err(|error| construction(format!("load: {error:?}")))?;

    // Resolve test ids through generic ports: space keys against the
    // thread's table snapshot, pad bits against the factory field map.
    let spaces = emu
        .memory_spaces()
        .map_err(|error| construction(format!("memory spaces: {error:?}")))?;
    let lookup_space = |key: &'static str| {
        spaces
            .iter()
            .find(|info| info.key == key)
            .map(|info| info.id)
            .ok_or_else(|| construction(format!("memory space missing: {key}")))
    };
    let lookup_field = |attachment: &'static str, control: &'static str| {
        field_map
            .get(&(
                AttachmentId::new(attachment),
                DigitalControlId::new(control),
            ))
            .copied()
    };
    let missing_field = |attachment: &'static str, control: &'static str| {
        construction(format!("test input field missing: {attachment}/{control}"))
    };
    let mut pad_fields = [[None; 8]; 2];
    for (pad, attachment) in [TEST_ATTACH_P1, TEST_ATTACH_P2].into_iter().enumerate() {
        for (bit, control) in TEST_PAD_BUTTONS.into_iter().enumerate() {
            // Pad 2 has no Select/Start buttons; the suite drives those
            // bits as no-ops (the device masks them too).
            let optional = pad == 1 && (bit == 2 || bit == 3);
            pad_fields[pad][bit] = match lookup_field(attachment, control) {
                Some(field) => Some(field),
                None if optional => None,
                None => return Err(missing_field(attachment, control)),
            };
        }
    }

    let tap = Arc::new(Mutex::new(Vec::new()));
    emu.tap_nominal_audio(Arc::clone(&tap))
        .map_err(|error| construction(format!("tap: {error:?}")))?;

    Ok(TestSystem {
        emu,
        gui_input,
        pad_fields,
        mic_field: Some(
            lookup_field(TEST_ATTACH_P2, TEST_MIC_CONTROL)
                .ok_or_else(|| missing_field(TEST_ATTACH_P2, TEST_MIC_CONTROL))?,
        ),
        work_ram: lookup_space(TEST_SPACE_WRAM_KEY)?,
        ppu_vram: lookup_space(TEST_SPACE_PPU_VRAM_KEY)?,
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
    /// Left Right per pad); fields resolve at open against the factory
    /// field map. Unknown hardware bits are skipped, but a field write
    /// failure is a loud error — never a silent no-op.
    pub fn sync_input(&mut self, pad1: u8, pad2: u8, mic: bool) -> Result<(), RomTestError> {
        let seed = |this: &mut Self, pad: usize, bits: u8| -> Result<(), RomTestError> {
            for bit in 0..8 {
                if let Some(field) = this.pad_fields[pad][bit] {
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
        if let Some(field) = self.mic_field {
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
    /// Space ids resolve once at open from the thread's table snapshot,
    /// so a table reorder can never silently mis-resolve.
    pub fn read_work_ram_byte(&self, addr: u32) -> Result<Option<u8>, RomTestError> {
        self.read_byte(self.work_ram, addr)
    }

    /// Read one byte from PPU VRAM through the thread inspect path.
    pub fn read_ppu_vram_byte(&self, addr: u32) -> Result<Option<u8>, RomTestError> {
        self.read_byte(self.ppu_vram, addr)
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
