use crate::{ConsoleError, ControllerInputs, NesInputFrame};
use nerust_contract_options::CoreOptions;
use nerust_contract_persistence::PersistenceTarget;
use nerust_contract_rom::RomIdentity;
use nerust_core::Core;
use nerust_core::controller::standard_controller::{
    Buttons, StandardController, StandardControllerSnapshot,
};
use nerust_screen_buffer::screen_buffer::ScreenBuffer;

/// Compatibility version for the console-owned wrapper around opaque core machine-state bytes.
///
/// Bump this only when the console wrapper schema or its validation rules change. The nested
/// `core_state` bytes remain owned by `nerust_core` and must continue to be treated as opaque by
/// the archive layer.
const LEGACY_CONSOLE_STATE_SCHEMA_VERSION: u32 = 1;
const CONSOLE_STATE_SCHEMA_VERSION: u32 = 2;
const STANDARD_CONTROLLER_MAX_INDEX: usize = 8;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreviewFrame {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StateExport {
    pub machine_state: Vec<u8>,
    pub preview: Option<PreviewFrame>,
    pub target: PersistenceTarget,
}

/// Console-owned runtime controller wrapper state.
///
/// This mirrors the current `StandardController` runtime semantics, including latched button bits,
/// shift indices, microphone state, and strobe mode.
#[derive(serde_derive::Serialize, serde_derive::Deserialize)]
struct ControllerPortStatePayload {
    input_bits: u32,
    index: u64,
}

#[derive(serde_derive::Serialize, serde_derive::Deserialize)]
struct ControllerAuxiliaryStatePayload {
    microphone: bool,
}

#[derive(serde_derive::Serialize, serde_derive::Deserialize)]
struct ControllerStatePayload {
    ports: [ControllerPortStatePayload; 2],
    auxiliary: ControllerAuxiliaryStatePayload,
    strobe: bool,
}

#[derive(serde_derive::Deserialize)]
struct LegacyControllerStatePayload {
    pad1_bits: u32,
    pad2_bits: u32,
    microphone: bool,
    index1: u64,
    index2: u64,
    strobe: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ControllerPortRuntimeState {
    inputs: ControllerInputs,
    index: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ControllerAuxiliaryRuntimeState {
    microphone: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ControllerRuntimeState {
    ports: [ControllerPortRuntimeState; 2],
    auxiliary: ControllerAuxiliaryRuntimeState,
    strobe: bool,
}

/// Console-owned save-state wrapper.
///
/// `core_state` is an opaque `nerust_core` machine-state payload stored without interpretation in
/// this crate. The console layer owns paused/frame counter/controller/source-frame restoration and
/// rejects wrapper schema mismatches before mutating any live console state.
#[derive(serde_derive::Serialize, serde_derive::Deserialize)]
struct ConsoleStatePayload {
    schema_version: u32,
    #[serde(with = "serde_bytes")]
    core_state: Vec<u8>,
    frame_counter: u64,
    paused: bool,
    controller: ControllerStatePayload,
    rom_identity: RomIdentity,
    options: CoreOptions,
    #[serde(with = "serde_bytes")]
    source_frame: Vec<u8>,
}

#[derive(serde_derive::Deserialize)]
struct LegacyConsoleStatePayload {
    schema_version: u32,
    #[serde(with = "serde_bytes")]
    core_state: Vec<u8>,
    frame_counter: u64,
    paused: bool,
    controller: LegacyControllerStatePayload,
    rom_identity: RomIdentity,
    options: CoreOptions,
    #[serde(with = "serde_bytes")]
    source_frame: Vec<u8>,
}

fn controller_inputs_from_buttons(buttons: Buttons) -> ControllerInputs {
    ControllerInputs::from_bits_retain(buttons.bits())
}

pub(crate) fn buttons_from_controller_inputs(inputs: ControllerInputs) -> Buttons {
    Buttons::from_bits_retain(inputs.bits())
}

pub(crate) fn buttons_from_nes_input_frame(frame: NesInputFrame) -> [Buttons; 2] {
    [
        buttons_from_controller_inputs(frame.player_one),
        buttons_from_controller_inputs(frame.player_two),
    ]
}

pub(crate) fn nes_input_frame_from_snapshot(snapshot: StandardControllerSnapshot) -> NesInputFrame {
    NesInputFrame {
        player_one: controller_inputs_from_buttons(snapshot.buttons[0]),
        player_two: controller_inputs_from_buttons(snapshot.buttons[1]),
        microphone: snapshot.microphone,
    }
}

fn controller_runtime_state_from_snapshot(
    snapshot: StandardControllerSnapshot,
) -> ControllerRuntimeState {
    ControllerRuntimeState {
        ports: [
            ControllerPortRuntimeState {
                inputs: controller_inputs_from_buttons(snapshot.buttons[0]),
                index: snapshot.index1,
            },
            ControllerPortRuntimeState {
                inputs: controller_inputs_from_buttons(snapshot.buttons[1]),
                index: snapshot.index2,
            },
        ],
        auxiliary: ControllerAuxiliaryRuntimeState {
            microphone: snapshot.microphone,
        },
        strobe: snapshot.strobe,
    }
}

fn controller_runtime_state_to_snapshot(
    state: ControllerRuntimeState,
) -> StandardControllerSnapshot {
    StandardControllerSnapshot {
        buttons: [
            buttons_from_controller_inputs(state.ports[0].inputs),
            buttons_from_controller_inputs(state.ports[1].inputs),
        ],
        microphone: state.auxiliary.microphone,
        index1: state.ports[0].index,
        index2: state.ports[1].index,
        strobe: state.strobe,
    }
}

fn controller_runtime_state_to_payload(state: ControllerRuntimeState) -> ControllerStatePayload {
    ControllerStatePayload {
        ports: [
            ControllerPortStatePayload {
                input_bits: u32::from(state.ports[0].inputs.bits()),
                index: state.ports[0].index as u64,
            },
            ControllerPortStatePayload {
                input_bits: u32::from(state.ports[1].inputs.bits()),
                index: state.ports[1].index as u64,
            },
        ],
        auxiliary: ControllerAuxiliaryStatePayload {
            microphone: state.auxiliary.microphone,
        },
        strobe: state.strobe,
    }
}

fn controller_snapshot_to_payload(snapshot: StandardControllerSnapshot) -> ControllerStatePayload {
    controller_runtime_state_to_payload(controller_runtime_state_from_snapshot(snapshot))
}

fn controller_runtime_state_from_payload(
    payload: &ControllerStatePayload,
) -> Result<ControllerRuntimeState, ConsoleError> {
    let decode_port = |payload: &ControllerPortStatePayload, label: &str| {
        let index = usize::try_from(payload.index)
            .map_err(|_| ConsoleError::Core(format!("controller {label} index overflow")))?;
        if index > STANDARD_CONTROLLER_MAX_INDEX {
            return Err(ConsoleError::Core(format!(
                "controller {label} index out of range"
            )));
        }
        Ok(ControllerPortRuntimeState {
            inputs: ControllerInputs::from_bits_retain(
                u8::try_from(payload.input_bits).map_err(|_| {
                    ConsoleError::Core(format!("controller {label} input overflow"))
                })?,
            ),
            index,
        })
    };
    Ok(ControllerRuntimeState {
        ports: [
            decode_port(&payload.ports[0], "port1")?,
            decode_port(&payload.ports[1], "port2")?,
        ],
        auxiliary: ControllerAuxiliaryRuntimeState {
            microphone: payload.auxiliary.microphone,
        },
        strobe: payload.strobe,
    })
}

fn controller_snapshot_from_payload(
    payload: &ControllerStatePayload,
) -> Result<StandardControllerSnapshot, ConsoleError> {
    controller_runtime_state_from_payload(payload).map(controller_runtime_state_to_snapshot)
}

fn controller_runtime_state_from_legacy_payload(
    payload: &LegacyControllerStatePayload,
) -> Result<ControllerRuntimeState, ConsoleError> {
    controller_runtime_state_from_payload(&ControllerStatePayload {
        ports: [
            ControllerPortStatePayload {
                input_bits: payload.pad1_bits,
                index: payload.index1,
            },
            ControllerPortStatePayload {
                input_bits: payload.pad2_bits,
                index: payload.index2,
            },
        ],
        auxiliary: ControllerAuxiliaryStatePayload {
            microphone: payload.microphone,
        },
        strobe: payload.strobe,
    })
}

fn validate_console_state_schema_version(version: u32, expected: u32) -> Result<(), ConsoleError> {
    if version == expected {
        Ok(())
    } else {
        Err(ConsoleError::Core(format!(
            "unsupported console state schema version: {version}"
        )))
    }
}

fn encode_console_state_payload(payload: &ConsoleStatePayload) -> Result<Vec<u8>, ConsoleError> {
    rmp_serde::to_vec_named(payload).map_err(|error| ConsoleError::Core(error.to_string()))
}

fn decode_console_state_payload(bytes: &[u8]) -> Result<ConsoleStatePayload, ConsoleError> {
    match rmp_serde::from_slice::<ConsoleStatePayload>(bytes) {
        Ok(payload) => {
            validate_console_state_schema_version(
                payload.schema_version,
                CONSOLE_STATE_SCHEMA_VERSION,
            )?;
            Ok(payload)
        }
        Err(current_error) => {
            let legacy =
                rmp_serde::from_slice::<LegacyConsoleStatePayload>(bytes).map_err(|_| {
                    ConsoleError::Core(format!("console state decode failed: {current_error}"))
                })?;
            validate_console_state_schema_version(
                legacy.schema_version,
                LEGACY_CONSOLE_STATE_SCHEMA_VERSION,
            )?;
            Ok(ConsoleStatePayload {
                schema_version: CONSOLE_STATE_SCHEMA_VERSION,
                core_state: legacy.core_state,
                frame_counter: legacy.frame_counter,
                paused: legacy.paused,
                controller: controller_runtime_state_to_payload(
                    controller_runtime_state_from_legacy_payload(&legacy.controller)?,
                ),
                rom_identity: legacy.rom_identity,
                options: legacy.options,
                source_frame: legacy.source_frame,
            })
        }
    }
}

fn validate_console_state_target(
    core: &Core,
    payload: &ConsoleStatePayload,
) -> Result<(), ConsoleError> {
    if payload.rom_identity != core.rom_identity() {
        return Err(ConsoleError::Core("console ROM identity mismatch".into()));
    }
    if payload.options != core.options() {
        return Err(ConsoleError::Core(
            "console runtime options mismatch".into(),
        ));
    }
    Ok(())
}

fn export_preview_frame(screen: &ScreenBuffer) -> Option<PreviewFrame> {
    let palette = screen
        .console_video_assets()
        .map(|assets| assets.palette_rgba8())?;
    let source_size = screen.source_logical_size();
    let mut indices = vec![0; screen.source_frame_len()];
    screen.copy_source_buffer(&mut indices);
    let mut rgba = Vec::with_capacity(indices.len() * 4);
    for index in indices {
        let palette_index = usize::from(index) * 4;
        let pixel = palette.get(palette_index..palette_index + 4)?;
        rgba.extend_from_slice(pixel);
    }
    Some(PreviewFrame {
        width: source_size.width as u32,
        height: source_size.height as u32,
        rgba,
    })
}

pub(crate) fn build_state_export(
    core: &Core,
    screen: &ScreenBuffer,
    controller: &StandardController,
    frame_counter: u64,
    paused: bool,
) -> Result<StateExport, ConsoleError> {
    let preview = export_preview_frame(screen);
    let machine_state = core
        .export_machine_state()
        .map_err(|error| ConsoleError::Core(error.to_string()))?;
    let source_frame = if screen.publishes_palette_frame() {
        let mut source_frame = vec![0; screen.source_frame_len()];
        screen.copy_source_buffer(&mut source_frame);
        source_frame
    } else {
        Vec::new()
    };
    let target = PersistenceTarget {
        rom_identity: core.rom_identity(),
        options: core.options(),
    };
    let state = ConsoleStatePayload {
        schema_version: CONSOLE_STATE_SCHEMA_VERSION,
        core_state: machine_state,
        frame_counter,
        paused,
        controller: controller_snapshot_to_payload(controller.export_snapshot()),
        rom_identity: target.rom_identity,
        options: target.options,
        source_frame,
    };
    Ok(StateExport {
        machine_state: encode_console_state_payload(&state)?,
        preview,
        target,
    })
}

pub(crate) fn restore_imported_state(
    core: &mut Core,
    screen: &mut ScreenBuffer,
    controller: &mut StandardController,
    frame_counter: &mut u64,
    paused: &mut bool,
    bytes: &[u8],
) -> Result<(), ConsoleError> {
    let payload = decode_console_state_payload(bytes)?;
    validate_console_state_target(core, &payload)?;
    let controller_snapshot = controller_snapshot_from_payload(&payload.controller)?;
    if screen.publishes_palette_frame()
        && !payload.source_frame.is_empty()
        && payload.source_frame.len() != screen.source_frame_len()
    {
        return Err(ConsoleError::Core(
            "console source frame length mismatch".into(),
        ));
    }
    core.import_machine_state(&payload.core_state)
        .map_err(|error| ConsoleError::Core(error.to_string()))?;
    if screen.publishes_palette_frame() && !payload.source_frame.is_empty() {
        screen.restore_source_buffer(&payload.source_frame);
    }
    controller.import_snapshot(controller_snapshot);
    *frame_counter = payload.frame_counter;
    *paused = payload.paused;
    Ok(())
}

#[cfg(test)]
mod tests;
