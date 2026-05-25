use crate::ConsoleError;
use crate::controller::{
    ControllerRuntime, StandardControllerState, encode_standard_controller_state,
};
use nerust_contract_options::CoreOptions;
use nerust_contract_persistence::PersistenceTarget;
use nerust_contract_rom::RomIdentity;
use nerust_core::Core;
use nerust_input_nes::frame::Buttons;
use nerust_screen_buffer::screen_buffer::ScreenBuffer;

/// Compatibility version for the console-owned wrapper around opaque core machine-state bytes.
///
/// Bump this only when the console wrapper schema or its validation rules change. The nested
/// `core_state` bytes remain owned by `nerust_core` and must continue to be treated as opaque by
/// the archive layer.
const LEGACY_CONSOLE_STATE_SCHEMA_VERSION: u32 = 1;
const STRUCTURED_CONSOLE_STATE_SCHEMA_VERSION: u32 = 2;
const CONSOLE_STATE_SCHEMA_VERSION: u32 = 3;
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

#[derive(serde_derive::Deserialize)]
struct LegacyControllerStatePayload {
    pad1_bits: u32,
    pad2_bits: u32,
    microphone: bool,
    index1: u64,
    index2: u64,
    strobe: bool,
}

#[derive(serde_derive::Serialize, serde_derive::Deserialize)]
struct StructuredControllerPortStatePayload {
    input_bits: u32,
    index: u64,
}

#[derive(serde_derive::Serialize, serde_derive::Deserialize)]
struct StructuredControllerAuxiliaryStatePayload {
    microphone: bool,
}

#[derive(serde_derive::Serialize, serde_derive::Deserialize)]
struct StructuredControllerStatePayload {
    ports: [StructuredControllerPortStatePayload; 2],
    auxiliary: StructuredControllerAuxiliaryStatePayload,
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
    #[serde(with = "serde_bytes")]
    controller_state: Vec<u8>,
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

#[derive(serde_derive::Serialize, serde_derive::Deserialize)]
struct StructuredConsoleStatePayload {
    schema_version: u32,
    #[serde(with = "serde_bytes")]
    core_state: Vec<u8>,
    frame_counter: u64,
    paused: bool,
    controller: StructuredControllerStatePayload,
    rom_identity: RomIdentity,
    options: CoreOptions,
    #[serde(with = "serde_bytes")]
    source_frame: Vec<u8>,
}

fn legacy_buttons(value: u32, label: &str) -> Result<Buttons, ConsoleError> {
    Ok(Buttons::from_bits_retain(u8::try_from(value).map_err(
        |_| ConsoleError::Core(format!("controller {label} input overflow")),
    )?))
}

fn legacy_index(value: u64, label: &str) -> Result<usize, ConsoleError> {
    let index = usize::try_from(value)
        .map_err(|_| ConsoleError::Core(format!("controller {label} index overflow")))?;
    if index > STANDARD_CONTROLLER_MAX_INDEX {
        return Err(ConsoleError::Core(format!(
            "controller {label} index out of range"
        )));
    }
    Ok(index)
}

fn legacy_controller_snapshot(
    payload: &LegacyControllerStatePayload,
) -> Result<StandardControllerState, ConsoleError> {
    Ok(StandardControllerState {
        buttons: [
            legacy_buttons(payload.pad1_bits, "port1")?,
            legacy_buttons(payload.pad2_bits, "port2")?,
        ],
        microphone: payload.microphone,
        index1: legacy_index(payload.index1, "port1")?,
        index2: legacy_index(payload.index2, "port2")?,
        strobe: payload.strobe,
    })
}

fn structured_controller_snapshot(
    payload: &StructuredControllerStatePayload,
) -> Result<StandardControllerState, ConsoleError> {
    Ok(StandardControllerState {
        buttons: [
            legacy_buttons(payload.ports[0].input_bits, "port1")?,
            legacy_buttons(payload.ports[1].input_bits, "port2")?,
        ],
        microphone: payload.auxiliary.microphone,
        index1: legacy_index(payload.ports[0].index, "port1")?,
        index2: legacy_index(payload.ports[1].index, "port2")?,
        strobe: payload.strobe,
    })
}

fn encode_console_state_payload(payload: &ConsoleStatePayload) -> Result<Vec<u8>, ConsoleError> {
    rmp_serde::to_vec_named(payload).map_err(|error| ConsoleError::Core(error.to_string()))
}

fn decode_console_state_payload(bytes: &[u8]) -> Result<ConsoleStatePayload, ConsoleError> {
    match rmp_serde::from_slice::<ConsoleStatePayload>(bytes) {
        Ok(payload) => {
            if payload.schema_version != CONSOLE_STATE_SCHEMA_VERSION {
                return Err(ConsoleError::Core(format!(
                    "unsupported console state schema version: {}",
                    payload.schema_version
                )));
            }
            Ok(payload)
        }
        Err(current_error) => {
            if let Ok(structured) = rmp_serde::from_slice::<StructuredConsoleStatePayload>(bytes) {
                if structured.schema_version != STRUCTURED_CONSOLE_STATE_SCHEMA_VERSION {
                    return Err(ConsoleError::Core(format!(
                        "unsupported console state schema version: {}",
                        structured.schema_version
                    )));
                }
                return Ok(ConsoleStatePayload {
                    schema_version: CONSOLE_STATE_SCHEMA_VERSION,
                    core_state: structured.core_state,
                    frame_counter: structured.frame_counter,
                    paused: structured.paused,
                    controller_state: encode_standard_controller_state(
                        structured_controller_snapshot(&structured.controller)?,
                    )
                    .map_err(ConsoleError::Core)?,
                    rom_identity: structured.rom_identity,
                    options: structured.options,
                    source_frame: structured.source_frame,
                });
            }
            let legacy =
                rmp_serde::from_slice::<LegacyConsoleStatePayload>(bytes).map_err(|_| {
                    ConsoleError::Core(format!("console state decode failed: {current_error}"))
                })?;
            if legacy.schema_version != LEGACY_CONSOLE_STATE_SCHEMA_VERSION {
                return Err(ConsoleError::Core(format!(
                    "unsupported console state schema version: {}",
                    legacy.schema_version
                )));
            }
            Ok(ConsoleStatePayload {
                schema_version: CONSOLE_STATE_SCHEMA_VERSION,
                core_state: legacy.core_state,
                frame_counter: legacy.frame_counter,
                paused: legacy.paused,
                controller_state: encode_standard_controller_state(legacy_controller_snapshot(
                    &legacy.controller,
                )?)
                .map_err(ConsoleError::Core)?,
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
    controller_state: Vec<u8>,
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
        controller_state,
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
    controller: &mut dyn ControllerRuntime,
    frame_counter: &mut u64,
    paused: &mut bool,
    bytes: &[u8],
) -> Result<(), ConsoleError> {
    let payload = decode_console_state_payload(bytes)?;
    validate_console_state_target(core, &payload)?;
    controller
        .validate_controller_state(&payload.controller_state)
        .map_err(ConsoleError::Core)?;
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
    controller
        .apply_controller_state(&payload.controller_state)
        .map_err(ConsoleError::Core)?;
    *frame_counter = payload.frame_counter;
    *paused = payload.paused;
    Ok(())
}

#[cfg(test)]
mod tests;
