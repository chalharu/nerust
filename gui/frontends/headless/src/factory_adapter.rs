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
    factory::{
        CoreFactory,
        settings::{FactorySettingsView, Language},
    },
};
use nerust_gui_shell::emu_core::EmuCore;
use nerust_input_traits::{GuiInput, InputAssignments, InputValue};
use nerust_render_traits::FrameBuffer;

use crate::{error::RomTestError, manifest::RomCase};

/// A loaded NES system ready for headless driving: session-owned
/// execution plus input writer and nominal-audio tap.
pub(crate) struct TestSystem {
    emu: EmuCore,
    gui_input: GuiInput,
    pad_buttons: Vec<Vec<(&'static str, usize)>>,
    spaces: Vec<SpaceInfo>,
    tap: Arc<Mutex<Vec<StereoSample>>>,
    channels: Vec<String>,
    serial_taps: std::collections::HashMap<String, Arc<Mutex<Vec<u8>>>>,
    /// Accepting factory's [`CoreFactory::display_name`]: the only
    /// honest system label (ROM probing decides, never the harness).
    system: &'static str,
}

/// Single construction site: every harness mode (validation,
/// measure) opens its session here, so case-to-system plumbing
/// (`options`, sample rate) cannot drift between modes. Driving
/// stays unified in `drive_case`; construction stays unified here.
pub(crate) fn open_case_system(
    factories: &[Box<dyn CoreFactory>],
    case: &RomCase,
    rom_bytes: &[u8],
) -> Result<TestSystem, RomTestError> {
    open_headless_system(
        factories,
        &case.id,
        rom_bytes,
        case.options.clone(),
        case.audio_sample_rate(),
    )
}

/// Build a loaded system through `CoreFactory` and wrap it in the
/// session execution engine.
///
/// The system is selected by ROM judgment: the first factory whose
/// `probe_media` accepts the bytes wins. No match is eligibility, not
/// failure — [`RomTestError::NoMatchingSystem`], mapped to an ignored
/// outcome upstream.
///
/// * Settings are factory defaults plus English, built here — not on
///   `CoreFactory` — because only headless driving needs them:
///   observed bytes never depend on presentation settings.
///   Case `options` parse through the factory's own CLI options schema.
/// * Both player slots get the P1 profile: the suite drives pad2 and
///   the microphone, which the factory default (P2 unassigned) lacks.
/// * Audio goes to a null backend stamped at the case rate; nominal
///   samples are tapped per stepped frame, never rate-controlled.
/// * After paused load the thread is silent until the first step, so
///   stepped frames start from a deterministic power-on frame zero.
pub(crate) fn open_headless_system(
    factories: &[Box<dyn CoreFactory>],
    case_id: &str,
    rom_bytes: &[u8],
    options: Vec<String>,
    audio_sample_rate: u32,
) -> Result<TestSystem, RomTestError> {
    let construction = |message: String| RomTestError::CoreConstruction {
        case_id: case_id.to_string(),
        message,
    };
    let media = nerust_core_traits::factory::load::MediaObject::new(None, rom_bytes.to_vec());
    let factory = factories
        .iter()
        .find(|factory| factory.probe_media(&media))
        .map(|factory| factory.as_ref())
        .ok_or_else(|| RomTestError::NoMatchingSystem {
            case_id: case_id.to_string(),
        })?;

    let view = headless_view(factory, case_id)?;
    // Case options pass straight through to the factory CLI schema
    // as argv (single-sourced flag spelling and value validation;
    // clap rejects typos loudly). Explicit options keep beating saved
    // settings inside `resolve_load_request`, as with real
    // command-line usage.
    let mut argv = Vec::with_capacity(options.len() + 1);
    argv.push("headless".to_string());
    argv.extend(options);
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
    emu.load_paused(&media, config.core_options)
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
    // Core-driven taps: one per channel the core lists, installed
    // without reading case data (open loads ROMs, never validates
    // tests). Unknown yaml channels fail at read time, never here.
    let channels = emu
        .output_channels()
        .map_err(|error| construction(format!("channels: {error:?}")))?;
    let mut serial_taps = std::collections::HashMap::new();
    for channel in &channels {
        let serial_tap = Arc::new(Mutex::new(Vec::new()));
        emu.tap_serial_output(channel, Arc::clone(&serial_tap))
            .map_err(|error| construction(format!("serial tap: {error:?}")))?;
        serial_taps.insert(channel.clone(), serial_tap);
    }

    Ok(TestSystem {
        emu,
        gui_input,
        pad_buttons,
        spaces,
        tap,
        channels,
        serial_taps,
        system: factory.display_name(),
    })
}

/// Deterministic settings view: factory defaults plus English,
/// independent of GUI/user state.
///
/// Lives in the headless frontend — not on `CoreFactory` — because it
/// is test tooling, not production interface: GUI frontends never call
/// it, and no system overrides it. Fails loudly when the factory
/// exposes no system defaults, so a missing seed surfaces instead of
/// silently shifting behavior.
fn headless_view(
    factory: &dyn CoreFactory,
    case_id: &str,
) -> Result<FactorySettingsView, RomTestError> {
    let system_config = factory
        .as_system_defaults()
        .and_then(|defaults| defaults.default_system_settings())
        .ok_or_else(|| RomTestError::CoreConstruction {
            case_id: case_id.to_string(),
            message: "factory exposes no system defaults".to_string(),
        })?;
    Ok(FactorySettingsView {
        language: Language::English,
        system_config: Some(system_config),
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
pub(crate) enum MemoryRead {
    Mapped(u8),
    OpenBus,
    Unmapped,
}

impl TestSystem {
    /// Accepting factory's display name (e.g. `"NES"`). Recorded at
    /// open so reports never guess the system from ROM bytes.
    pub fn system_name(&self) -> &'static str {
        self.system
    }

    /// Driving capability: stepping, input, reset. Sees neither taps,
    /// spaces, nor channels — enforced by construction (only these
    /// fields cross the boundary). Control and observe views each
    /// need `&mut` (input publish, framebuffer swap), so they are
    /// sequential, never simultaneous; inspection is fully shared.
    pub(crate) fn control(&mut self) -> SystemControl<'_> {
        SystemControl {
            emu: &self.emu,
            gui_input: &mut self.gui_input,
            pad_buttons: &self.pad_buttons,
        }
    }

    /// Per-frame observation: screen, audio, channel taps. Sees no
    /// input and no spaces — only what a frame publishes.
    pub(crate) fn observe(&mut self) -> FrameObserve<'_> {
        FrameObserve {
            emu: &mut self.emu,
            tap: &self.tap,
            channels: &self.channels,
            serial_taps: &self.serial_taps,
        }
    }

    /// Point inspection: memory and registers through the paused
    /// thread. Fully shared — inspection never mutates session state,
    /// so holders can coexist with each other (but not with a live
    /// control/observe borrow).
    pub(crate) fn inspect(&self) -> SystemInspector<'_> {
        SystemInspector {
            emu: &self.emu,
            spaces: &self.spaces,
        }
    }
}

/// Driving capability: stepping, input, reset. Owns no session state
/// beyond its three fields; in particular it cannot read taps, spaces,
/// or channels, so driving code provably cannot observe.
pub(crate) struct SystemControl<'a> {
    emu: &'a EmuCore,
    gui_input: &'a mut GuiInput,
    pad_buttons: &'a [Vec<(&'static str, usize)>],
}

impl SystemControl<'_> {
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

    /// Reset emulation to a deterministic frame zero.
    pub fn reset(&self) -> Result<(), RomTestError> {
        self.emu
            .reset()
            .map_err(|error| RomTestError::EmuThread(format!("reset: {error:?}")))
    }
}

/// Per-frame observation: screen, audio, channel taps. Cannot drive
/// input or reset, and cannot resolve spaces — observation code
/// provably cannot steer the session it measures.
pub(crate) struct FrameObserve<'a> {
    emu: &'a mut EmuCore,
    tap: &'a Mutex<Vec<StereoSample>>,
    channels: &'a [String],
    serial_taps: &'a HashMap<String, Arc<Mutex<Vec<u8>>>>,
}

impl FrameObserve<'_> {
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

    /// Channel names the core listed at open (possibly none). A
    /// borrowed snapshot — iterating it costs nothing per frame.
    pub fn channel_names(&self) -> &[String] {
        self.channels
    }

    /// Drain one channel tap (this frame's fresh bytes only).
    /// Accumulation is the caller's job: the tap holds the latest
    /// frame's delta, mirroring the audio tap. Unknown channels fail
    /// loudly here (read-time validation, like registers/spaces) —
    /// never silently empty.
    pub fn drain_channel(&self, channel: &str) -> Result<Vec<u8>, RomTestError> {
        let tap = self.serial_taps.get(channel).ok_or_else(|| {
            RomTestError::EmuThread(format!(
                "unknown output channel `{channel}` (core lists {:?})",
                self.channels
            ))
        })?;
        tap.lock()
            .map(|mut guard| std::mem::take(&mut *guard))
            .map_err(|error| RomTestError::EmuThread(format!("serial tap lock: {error}")))
    }
}

/// Point inspection: memory and registers through the paused thread.
/// Shared borrow throughout — inspection observes but never steers.
pub(crate) struct SystemInspector<'a> {
    emu: &'a EmuCore,
    spaces: &'a [SpaceInfo],
}

impl SystemInspector<'_> {
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

    /// Read the live register list through the thread inspect path.
    /// Same paused moment as a memory dump: the caller stepped first,
    /// so names and values are frame-addressed. Empty rows keep the
    /// response to registers only.
    pub fn read_registers(&self) -> Result<Vec<(&'static str, u64)>, RomTestError> {
        let inner = self
            .emu
            .inspect(InspectRequest {
                space: None,
                addr: None,
                rows: 0,
            })
            .map_err(|error| RomTestError::EmuThread(format!("transport: {error:?}")))?
            .map_err(|error| RomTestError::EmuThread(format!("inspect: {error:?}")))?;
        Ok(inner.registers.to_vec())
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_factories_yield_no_matching_system() {
        // Eligibility without any system: no ROM read, no thread, no
        // emulation — the probe loop over zero factories decides.
        match open_headless_system(&[], "case", b"NES\x1Ajunk", Vec::new(), 48_000) {
            Err(RomTestError::NoMatchingSystem { .. }) => {}
            Err(error) => panic!("unexpected error: {error:?}"),
            Ok(_) => panic!("empty factories must not open a system"),
        }
    }
}
