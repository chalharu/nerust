//! Single seam to system construction and session-layer driving.
//! System addressing resolves through pre-existing maps only.

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
use nerust_input_traits::{InputAssignments, InputValue};
use nerust_render_traits::FrameBuffer;

use crate::{error::RomTestError, manifest::RomCase};

/// A loaded NES system ready for headless driving: session-owned
/// execution plus input writer and nominal-audio tap.
pub(crate) struct TestSystem {
    emu: EmuCore,
    gui_input: Box<dyn nerust_core_traits::factory::GuiInputPort>,
    pad_buttons: Vec<Vec<(&'static str, usize)>>,
    spaces: Vec<SpaceInfo>,
    tap: Arc<Mutex<Vec<StereoSample>>>,
    channels: Vec<String>,
    serial_taps: std::collections::HashMap<String, Arc<Mutex<Vec<u8>>>>,
    /// Accepting factory's [`CoreFactory::display_name`]: the only
    /// honest system label (ROM probing decides, never the harness).
    system: &'static str,
}

/// Single construction site: every harness mode opens its session here, so
/// case-to-system plumbing cannot drift between modes.
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

/// Build a loaded system through `CoreFactory` and wrap it in the session engine.
/// ROM judgment selects the system; no match is eligibility, not failure.
pub(crate) fn open_headless_system(
    factories: &[Box<dyn CoreFactory>],
    case_id: &str,
    rom_bytes: &[u8],
    options: Vec<String>,
    audio_sample_rate: u32,
) -> Result<TestSystem, RomTestError> {
    let factory = probe_factory(factories, case_id, rom_bytes)?;
    let view = headless_view(factory, case_id)?;
    let resolved = resolve_case_options(factory, case_id, &view, &options)?;
    let parts = create_core_parts(factory, case_id, &view, resolved, audio_sample_rate)?;
    assemble_test_system(factory, case_id, parts, rom_bytes)
}

/// Single construction error mapper: every failure carries the case id.
fn construction(case_id: &str, message: String) -> RomTestError {
    RomTestError::CoreConstruction {
        case_id: case_id.to_string(),
        message,
    }
}

/// Probe the factory table: first match owns the ROM, no match is eligibility.
fn probe_factory<'a>(
    factories: &'a [Box<dyn CoreFactory>],
    case_id: &str,
    rom_bytes: &[u8],
) -> Result<&'a dyn CoreFactory, RomTestError> {
    let media = nerust_core_traits::factory::load::MediaObject::new(None, rom_bytes.to_vec());
    factories
        .iter()
        .find(|factory| factory.probe_media(&media))
        .map(|factory| factory.as_ref())
        .ok_or_else(|| RomTestError::NoMatchingSystem {
            case_id: case_id.to_string(),
        })
}

/// Resolve case options through the factory CLI schema as argv.
/// Explicit options beat saved settings inside `resolve_load_request`.
fn resolve_case_options(
    factory: &dyn CoreFactory,
    case_id: &str,
    view: &FactorySettingsView,
    options: &[String],
) -> Result<nerust_core_traits::factory::load::ResolvedLoadRequest, RomTestError> {
    let mut argv = Vec::with_capacity(options.len() + 1);
    argv.push("headless".to_string());
    argv.extend(options.iter().cloned());
    let load_options = {
        let schema = factory.load_options_schema();
        let matches = schema
            .augment_args(clap::Command::new("headless"))
            .try_get_matches_from(argv)
            .map_err(|error| construction(case_id, format!("test option: {error}")))?;
        schema
            .arg_matches(&matches)
            .map_err(|error| construction(case_id, format!("test option: {error}")))?
    };
    factory
        .resolve_load_request(view, load_options)
        .map_err(|error| construction(case_id, format!("resolve options: {error:?}")))
}

/// Create the core plus adapter against the factory default assignments.
/// P2 runs the P1 profile clone: the suite drives both pads plus the microphone.
fn create_core_parts(
    factory: &dyn CoreFactory,
    case_id: &str,
    view: &FactorySettingsView,
    resolved: nerust_core_traits::factory::load::ResolvedLoadRequest,
    audio_sample_rate: u32,
) -> Result<LoadedCoreParts, RomTestError> {
    let base = factory.input_system_factory().default_assignments();
    let assignments = duplicate_p1_to_p2(&base).ok_or_else(|| {
        construction(case_id, "factory default assigns no P1 profile".to_string())
    })?;
    let speaker: Box<dyn AudioBackend> = Box::new(NullAudioBackend(audio_sample_rate));
    let parts = factory
        .create_core_and_adapter_with_assignments(view, speaker, &assignments)
        .map_err(|error| construction(case_id, format!("create_core: {error:?}")))?;
    Ok(LoadedCoreParts {
        parts,
        assignments,
        options: Some(resolved.options),
    })
}

/// Loaded core plus assignments plus resolved options for assembly.
struct LoadedCoreParts {
    parts: nerust_core_traits::factory::CoreParts,
    assignments: InputAssignments,
    options: Option<Box<dyn nerust_core_traits::CoreOptions>>,
}

/// Assemble the session engine: paused load, table-driven pad map, core taps.
fn assemble_test_system(
    factory: &dyn CoreFactory,
    case_id: &str,
    core: LoadedCoreParts,
    rom_bytes: &[u8],
) -> Result<TestSystem, RomTestError> {
    let LoadedCoreParts {
        parts,
        assignments,
        options,
    } = core;
    let config = CoreConfig {
        region: None,
        bios_paths: HashMap::new(),
        controllers: HashMap::new(),
        core_options: options,
        audio_sample_rate: None,
    };
    let (emu, gui_input, field_map, _) = EmuCore::from_parts(parts);
    // Paused load: power-on state survives to frame zero (no free-run
    // frames, no compensating reset that would destroy it).
    let media = nerust_core_traits::factory::load::MediaObject::new(None, rom_bytes.to_vec());
    emu.load_paused(&media, config.core_options)
        .map_err(|error| construction(case_id, format!("load: {error:?}")))?;
    // Resolve test ids through the injected map, never from positions.
    // Space roles against the table snapshot, pad bits against the field map.
    let spaces = emu
        .memory_spaces()
        .map_err(|error| construction(case_id, format!("memory spaces: {error:?}")))?;
    let pad_buttons = resolve_pad_buttons(case_id, &assignments, &field_map)?;
    let (tap, channels, serial_taps) = attach_taps(case_id, &emu)?;
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

/// Pad buttons resolve through the slot profiles to buffer fields.
/// Present but unmapped is loud drift; digital controls resolve directly.
fn resolve_pad_buttons(
    case_id: &str,
    assignments: &InputAssignments,
    field_map: &nerust_input_traits::UnifiedFieldMap,
) -> Result<Vec<Vec<(&'static str, usize)>>, RomTestError> {
    let mut pad_buttons = Vec::with_capacity(assignments.slots().len());
    for (pad, (attachment, profile)) in assignments.slots().iter().enumerate() {
        let profile = profile.as_ref().ok_or_else(|| {
            construction(case_id, format!("slot {pad} assigns no controller profile"))
        })?;
        let group = profile
            .port_groups()
            .get(pad)
            .ok_or_else(|| construction(case_id, format!("slot {pad} has no control group")))?;
        let mut buttons = Vec::with_capacity(group.len());
        for info in group.iter() {
            let Some(control_id) = info.id.as_digital() else {
                continue;
            };
            let field = nerust_core_traits::factory::resolve_unified_field(
                field_map,
                *attachment,
                control_id,
            )
            .ok_or_else(|| {
                construction(
                    case_id,
                    format!("test input field missing: {attachment}/{control_id}"),
                )
            })?;
            buttons.push((control_id.as_str(), field));
        }
        pad_buttons.push(buttons);
    }
    Ok(pad_buttons)
}

/// Attached output taps: nominal audio, channel names, and serial taps.
type AttachedTaps = (
    Arc<Mutex<Vec<StereoSample>>>,
    Vec<String>,
    std::collections::HashMap<String, Arc<Mutex<Vec<u8>>>>,
);

/// Core-driven taps: nominal audio plus one serial tap per listed channel.
/// Unknown yaml channels fail at read time, never here.
fn attach_taps(case_id: &str, emu: &EmuCore) -> Result<AttachedTaps, RomTestError> {
    let tap = Arc::new(Mutex::new(Vec::new()));
    emu.tap_nominal_audio(Arc::clone(&tap))
        .map_err(|error| construction(case_id, format!("tap: {error:?}")))?;
    let channels = emu
        .output_channels()
        .map_err(|error| construction(case_id, format!("channels: {error:?}")))?;
    let mut serial_taps = std::collections::HashMap::new();
    for channel in &channels {
        let serial_tap = Arc::new(Mutex::new(Vec::new()));
        emu.tap_serial_output(channel, Arc::clone(&serial_tap))
            .map_err(|error| construction(case_id, format!("serial tap: {error:?}")))?;
        serial_taps.insert(channel.clone(), serial_tap);
    }
    Ok((tap, channels, serial_taps))
}

/// Deterministic settings view: factory defaults plus English.
/// Test tooling, not production interface; fails loudly without defaults.
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

/// Clone the P1 profile onto P2: the suite drives both pads.
fn duplicate_p1_to_p2(base: &InputAssignments) -> Option<InputAssignments> {
    let p1 = base.slots().first()?.1.clone()?;
    let mut slots = base.slots().to_vec();
    if slots.len() > 1 {
        slots[1].1 = Some(p1);
    }
    Some(InputAssignments::new(slots))
}

/// One address read through the Spaces table: mapped value, open bus on
/// covered-but-unreadable addresses, or unmapped.
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
    /// spaces, nor channels — enforced by construction.
    pub(crate) fn control(&mut self) -> SystemControl<'_> {
        SystemControl {
            emu: &self.emu,
            gui_input: &mut *self.gui_input,
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

    /// Point inspection through the paused thread. Fully shared; inspection
    /// never mutates session state.
    pub(crate) fn inspect(&self) -> SystemInspector<'_> {
        SystemInspector {
            emu: &self.emu,
            spaces: &self.spaces,
        }
    }
}

/// Driving capability: stepping, input, reset. Owns no session state beyond
/// its three fields; driving code provably cannot observe.
pub(crate) struct SystemControl<'a> {
    emu: &'a EmuCore,
    gui_input: &'a mut (dyn nerust_core_traits::factory::GuiInputPort + 'static),
    pad_buttons: &'a [Vec<(&'static str, usize)>],
}

impl SystemControl<'_> {
    /// Drive one button: strict pad map, missing controls error loudly.
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
        let Some(field) = buttons
            .iter()
            .find(|(id, _)| *id == control)
            .map(|(_, field)| *field)
        else {
            let known: Vec<&str> = buttons.iter().map(|(id, _)| *id).collect();
            return Err(RomTestError::EmuThread(format!(
                "unknown button `{control}` for pad {pad} (known: {known:?})"
            )));
        };
        self.gui_input
            .set(field, InputValue::Digital(pressed))
            .map_err(|error| RomTestError::EmuThread(format!("seed input: {error}")))?;
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

    /// Reset emulation to a deterministic frame zero.
    pub fn reset(&self) -> Result<(), RomTestError> {
        self.emu
            .reset()
            .map_err(|error| RomTestError::EmuThread(format!("reset: {error:?}")))
    }
}

/// Per-frame observation: screen, audio, channel taps. Cannot drive input
/// or reset, and cannot resolve spaces.
pub(crate) struct FrameObserve<'a> {
    emu: &'a mut EmuCore,
    tap: &'a Mutex<Vec<StereoSample>>,
    channels: &'a [String],
    serial_taps: &'a HashMap<String, Arc<Mutex<Vec<u8>>>>,
}

impl FrameObserve<'_> {
    /// Latest published frame, read from the shared display buffer.
    /// Swaps first; valid after a step (the thread is paused).
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
    /// Accumulation is the caller's job; unknown channels fail loudly here.
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
    /// Read one byte through the thread inspect path, resolved against the table.
    /// Covered-but-unreadable means open bus; uncovered means unmapped.
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
    /// Same paused moment as a memory dump; empty rows stay registers-only.
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

/// Silent audio backend stamped at the case rate. Nominal samples stay
/// comparable to expectations.
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
    use nerust_input_traits::AttachmentId;

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

    #[test]
    fn construction_error_carries_case_id() {
        match construction("case-7", "boom".to_string()) {
            RomTestError::CoreConstruction { case_id, message } => {
                assert_eq!(case_id, "case-7");
                assert_eq!(message, "boom");
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[test]
    fn duplicate_p1_needs_a_profile_to_clone() {
        use nerust_input_traits::InputAssignments;
        // Empty table: nothing to clone.
        assert!(duplicate_p1_to_p2(&InputAssignments::new(Vec::new())).is_none());
        // P1 without a profile: no source, no clone.
        let attachment = AttachmentId::new("test.pad");
        let lonely = InputAssignments::new(vec![(attachment, None)]);
        assert!(duplicate_p1_to_p2(&lonely).is_none());
    }

    #[test]
    fn pad_map_resolves_empty_and_rejects_unprofiled_slots() {
        use nerust_input_traits::InputAssignments;
        // Empty table resolves to an empty map.
        let empty = resolve_pad_buttons(
            "case",
            &InputAssignments::new(Vec::new()),
            &Default::default(),
        )
        .expect("empty table resolves");
        assert!(empty.is_empty());
        // A slot without a profile fails loudly, naming the slot.
        let attachment = AttachmentId::new("test.pad");
        let unprofiled = InputAssignments::new(vec![(attachment, None)]);
        match resolve_pad_buttons("case", &unprofiled, &Default::default()) {
            Err(RomTestError::CoreConstruction { case_id, message }) => {
                assert_eq!(case_id, "case");
                assert!(
                    message.contains("slot 0 assigns no controller profile"),
                    "{message}"
                );
            }
            Err(other) => panic!("unexpected error: {other:?}"),
            Ok(_) => panic!("unprofiled slot must fail"),
        }
    }
}
