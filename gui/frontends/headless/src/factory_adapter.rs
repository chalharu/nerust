//! Single seam to system construction and session-layer driving.
//!
//! The generic validation harness drives [`TestSystem`], which owns the
//! session execution engine ([`EmuCore`]) built from factory parts. This
//! module names no system-concrete types: [`open_headless_system`] takes
//! `&dyn CoreFactory`, and all system addressing resolves through
//! pre-existing maps — the CLI options schema, the debugger space
//! table, and the input field map. Production code carries no test
//! support; the harness owns only its test-domain vocabulary (suite
//! bit order, assertion spaces).

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
use nerust_input_traits::{AbstractKey, GuiInput, InputAssignments, InputValue};
use nerust_render_traits::FrameBuffer;

use crate::{error::RomTestError, manifest::Mmc3IrqVariant};

/// Suite pad bit order as system-agnostic logical keys: bit `i` of a
/// pad byte drives the control keyed `SUITE_PAD_KEYS[i]` in the slot's
/// profile group. Bit layouts are the suite's own event vocabulary.
const SUITE_PAD_KEYS: [AbstractKey; 8] = [
    AbstractKey::Button1,
    AbstractKey::Button2,
    AbstractKey::Select,
    AbstractKey::Start,
    AbstractKey::DpadUp,
    AbstractKey::DpadDown,
    AbstractKey::DpadLeft,
    AbstractKey::DpadRight,
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

/// Build a loaded system through `CoreFactory` and wrap it in the
/// session execution engine.
///
/// * Settings come from the factory's headless view; test options
///   (`mmc3_irq_variant`, …) parse through the factory's own CLI
///   options schema, exactly like real command-line usage.
/// * Both player slots get the P1 profile: the suite drives pad2 and
///   the microphone, which the factory default (P2 unassigned) lacks.
/// * Audio goes to a null backend stamped at the case rate; nominal
///   samples are tapped per stepped frame, never rate-controlled.
/// * After paused load the thread is silent until the first step, so
///   stepped frames start from a deterministic power-on frame zero.
pub fn open_headless_system(
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
    // Test options parse through the factory's own CLI schema, so the
    // flag spelling and value validation stay single-sourced in the
    // factory. Explicit options keep beating saved settings inside
    // `resolve_load_request`, as with real command-line usage.
    let load_options = match mmc3_irq_variant.map(|variant| match variant {
        Mmc3IrqVariant::Sharp => "sharp",
        Mmc3IrqVariant::Nec => "nec",
    }) {
        None => factory.default_load_options(),
        Some(value) => {
            let schema = factory.load_options_schema();
            let matches = schema
                .augment_args(clap::Command::new("headless"))
                .try_get_matches_from(["headless", "--mmc3-irq-variant", value])
                .map_err(|error| construction(format!("test option: {error}")))?;
            schema
                .arg_matches(&matches)
                .map_err(|error| construction(format!("test option: {error}")))?
        }
    };
    let resolved = factory
        .resolve_load_request(&view, load_options)
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

    // Resolve test ids through the injected map: space roles against
    // the thread's table snapshot, pad bits against the field map.
    // Ids always come from live system state, never from positions.
    let spaces = emu
        .memory_spaces()
        .map_err(|error| construction(format!("memory spaces: {error:?}")))?;
    // The space containing an address is unique (tables forbid
    // overlap), so assertions resolve by address against the live
    // snapshot — no role or key inventory needed.
    let lookup_space = |addr: u32| {
        spaces
            .iter()
            .find(|info| info.range.contains(&addr))
            .map(|info| info.id)
            .ok_or_else(|| construction(format!("no memory space contains: {addr:#X}")))
    };
    // Pad bits resolve through the slot profiles: bit `i` drives the
    // control keyed `SUITE_PAD_KEYS[i]` in the slot's group, and the
    // field map turns that control into a buffer field. Absent from
    // the group means no such hardware (a silent no-op, as the device
    // masks it too); present but unmapped is loud drift.
    let mut pad_fields = [[None; 8]; 2];
    if assignments.slots.len() != pad_fields.len() {
        return Err(construction("slot count exceeds pad capacity".to_string()));
    }
    let mut mic_candidates = Vec::new();
    for (pad, (attachment, profile)) in assignments.slots.iter().enumerate() {
        let profile = profile
            .as_ref()
            .ok_or_else(|| construction(format!("slot {pad} assigns no controller profile")))?;
        let group = profile
            .port_groups()
            .get(pad)
            .ok_or_else(|| construction(format!("slot {pad} has no control group")))?;
        for (bit, key) in SUITE_PAD_KEYS.iter().enumerate() {
            let control = match group.iter().find(|info| info.abstract_key == Some(*key)) {
                None => continue,
                Some(info) => info.id,
            };
            pad_fields[pad][bit] = Some(
                field_map
                    .get(&(*attachment, control))
                    .copied()
                    .ok_or_else(|| {
                        construction(format!("test input field missing: {attachment}/{control}"))
                    })?,
            );
        }
        for info in group.iter().filter(|info| info.abstract_key.is_none()) {
            mic_candidates.push(field_map.get(&(*attachment, info.id)).copied().ok_or_else(
                || {
                    construction(format!(
                        "test input field missing: {attachment}/{}",
                        info.id
                    ))
                },
            )?);
        }
    }
    // Unbound controls (no system-agnostic key) are special hardware;
    // exactly one is the microphone. Zero means none attached, more
    // than one is ambiguous — both loud, never guessed.
    let mic_field = match mic_candidates[..] {
        [] => None,
        [field] => Some(field),
        _ => return Err(construction("ambiguous microphone controls".to_string())),
    };

    let tap = Arc::new(Mutex::new(Vec::new()));
    emu.tap_nominal_audio(Arc::clone(&tap))
        .map_err(|error| construction(format!("tap: {error:?}")))?;

    Ok(TestSystem {
        emu,
        gui_input,
        pad_fields,
        mic_field,
        // Pinned at open from the live snapshot: the suite's canonical
        // representatives for each asserted space. Fail fast here
        // rather than on the first assertion.
        work_ram: lookup_space(0x0000)?,
        ppu_vram: lookup_space(0x2000)?,
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
