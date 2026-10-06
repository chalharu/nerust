//! Single seam to system construction and session-layer driving.
//!
//! The generic validation harness drives [`TestSystem`], which owns the
//! session execution engine ([`EmuCore`]) built from factory parts. This
//! module names no system-concrete types: [`open_headless_system`] takes
//! `&dyn CoreFactory`, and all system addressing resolves through
//! pre-existing maps — the CLI options schema, the debugger space
//! table, and the input field map. Production code carries no test
//! support; the harness owns only its test-domain vocabulary (event
//! actions, assertion kinds).

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use nerust_core_traits::{
    CoreConfig,
    audio::{AudioBackend, StereoSample},
    debugger::{InspectRequest, SpaceInfo, StepUnit},
    factory::CoreFactory,
};
use nerust_gui_shell::emu_core::EmuCore;
use nerust_input_traits::{GuiInput, InputAssignments, InputValue};
use nerust_render_traits::FrameBuffer;

use crate::error::RomTestError;

/// A loaded NES system ready for headless driving: session-owned
/// execution plus input writer and nominal-audio tap.
pub struct TestSystem {
    emu: EmuCore,
    gui_input: GuiInput,
    pad_buttons: Vec<Vec<(&'static str, usize)>>,
    spaces: Vec<SpaceInfo>,
    tap: Arc<Mutex<Vec<StereoSample>>>,
}

/// Build a loaded system through `CoreFactory` and wrap it in the
/// session execution engine.
///
/// * Settings come from the factory's headless view; case `options`
///   parse through the factory's own CLI options schema.
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
    options: Vec<String>,
    audio_sample_rate: u32,
) -> Result<TestSystem, RomTestError> {
    let construction = |message: String| RomTestError::CoreConstruction {
        case_id: case_id.to_string(),
        message,
    };

    let view = factory
        .headless_view()
        .map_err(|error| construction(format!("headless view: {error:?}")))?;
    // Case options split in the manifest: schema argv parses through
    // the factory's own CLI schema (single-sourced flag spelling and
    // value validation; clap rejects typos loudly), while harness ROM
    // overrides never reach the core. Explicit options keep beating
    // saved settings inside `resolve_load_request`, as with real
    // command-line usage.
    let (argv_options, _) = crate::manifest::split_case_options(case_id, &options)?;
    let mut argv = Vec::with_capacity(argv_options.len() + 1);
    argv.push("headless".to_string());
    argv.extend(argv_options);
    let load_options = {
        let schema = factory.load_options_schema();
        let matches = schema
            .augment_args(clap::Command::new("headless"))
            .try_get_matches_from(argv)
            .map_err(|error| construction(format!("test option: {error}")))?;
        schema
            .arg_matches(&matches)
            .map_err(|error| construction(format!("test option: {error}")))?
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
    // Pad buttons resolve through the slot profiles: every control in
    // the slot's group maps through the field map to a buffer field —
    // addressed by the control id string the profile itself exposes.
    // Absent from the group means no such hardware (a silent no-op
    // when driven, as the device masks it too); present but unmapped
    // is loud drift.
    let mut pad_buttons = Vec::with_capacity(assignments.slots.len());
    for (pad, (attachment, profile)) in assignments.slots.iter().enumerate() {
        let profile = profile
            .as_ref()
            .ok_or_else(|| construction(format!("slot {pad} assigns no controller profile")))?;
        let group = profile
            .port_groups()
            .get(pad)
            .ok_or_else(|| construction(format!("slot {pad} has no control group")))?;
        let mut buttons = Vec::with_capacity(group.len());
        for info in group.iter() {
            let field = field_map
                .get(&(*attachment, info.id))
                .copied()
                .ok_or_else(|| {
                    construction(format!(
                        "test input field missing: {attachment}/{}",
                        info.id
                    ))
                })?;
            buttons.push((info.id.as_str(), field));
        }
        pad_buttons.push(buttons);
    }

    let tap = Arc::new(Mutex::new(Vec::new()));
    emu.tap_nominal_audio(Arc::clone(&tap))
        .map_err(|error| construction(format!("tap: {error:?}")))?;

    Ok(TestSystem {
        emu,
        gui_input,
        pad_buttons,
        spaces,
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

/// One address read through the Spaces table: a mapped value, an
/// open (floating) bus on covered-but-unreadable addresses, or an
/// unmapped address covered by no space.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemoryRead {
    Mapped(u8),
    OpenBus,
    Unmapped,
}

impl TestSystem {
    /// Drive one button for the next frame, addressed by the control
    /// id string the slot profile exposes (e.g. `"nes.control.a"`).
    /// Unknown to every pad means a typo: loud error. Known but absent
    /// from this pad means no such hardware: silent no-op, as the
    /// device masks it too. A field write failure is always loud —
    /// never a silent no-op.
    pub fn set_button(
        &mut self,
        pad: usize,
        control: &str,
        pressed: bool,
    ) -> Result<(), RomTestError> {
        let buttons = self
            .pad_buttons
            .get(pad)
            .ok_or_else(|| RomTestError::EmuThread(format!("no such pad: {pad}")))?;
        if let Some(field) = buttons
            .iter()
            .find(|(id, _)| *id == control)
            .map(|(_, field)| *field)
        {
            self.gui_input
                .state
                .set(field, InputValue::Digital(pressed))
                .map_err(|error| RomTestError::EmuThread(format!("seed input: {error}")))?;
            self.gui_input.publish();
            return Ok(());
        }
        let known = self
            .pad_buttons
            .iter()
            .flatten()
            .any(|(id, _)| *id == control);
        if known {
            Ok(())
        } else {
            Err(RomTestError::EmuThread(format!(
                "unknown button: {control}"
            )))
        }
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

    /// Read one byte through the thread inspect path, resolved by
    /// address against the table snapshot (tables forbid overlap, so
    /// the home is unique). Covered-but-unreadable means open bus;
    /// uncovered means unmapped — distinguished here, never guessed.
    pub fn read_memory_byte(&self, addr: u32) -> Result<MemoryRead, RomTestError> {
        let Some(space) = self
            .spaces
            .iter()
            .find(|info| info.range.contains(&addr))
            .map(|info| info.id)
        else {
            return Ok(MemoryRead::Unmapped);
        };
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
            .map(|row| MemoryRead::Mapped(row.bytes[0]))
            .unwrap_or(MemoryRead::OpenBus))
    }

    /// Reset emulation to a deterministic frame zero.
    pub fn reset(&self) -> Result<(), RomTestError> {
        self.emu
            .reset()
            .map_err(|error| RomTestError::EmuThread(format!("reset: {error:?}")))
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
