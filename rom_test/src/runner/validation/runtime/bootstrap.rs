use super::ValidationRuntime;
use crate::error::RomTestError;
use crate::manifest::RomCase;
use crate::media::{HashingMixer, validation_screen_buffer};
use nerust_cartridge_data::parse_cartridge_bytes;
use nerust_input_nes::frame::Buttons;
use nerust_input_nes_runtime::nes_input_cell::{NesInputCell, SharedNesInputCell};
use nerust_nes_core::Core;
use nerust_nes_device::nes_pad::NesPadDevice;
use std::sync::Arc;

impl ValidationRuntime {
    pub(in crate::runner::validation) fn new(
        case: &RomCase,
        rom_bytes: &[u8],
    ) -> Result<Self, RomTestError> {
        let cartridge_data =
            parse_cartridge_bytes(rom_bytes).map_err(|error| RomTestError::CoreConstruction {
                case_id: case.id.clone(),
                message: error.to_string(),
            })?;
        let core =
            Core::new_with_options(cartridge_data, case.core_options()).map_err(|error| {
                RomTestError::CoreConstruction {
                    case_id: case.id.clone(),
                    message: error.to_string(),
                }
            })?;

        let cell = Arc::new(NesInputCell::new());
        Ok(Self {
            screen_buffer: validation_screen_buffer(),
            core,
            controller: NesPadDevice::new(SharedNesInputCell(cell.clone())),
            cell,
            backend: HashingMixer::new(case.audio_sample_rate()),
            frame_counter: 0,
            pad1: Buttons::empty(),
            pad2: Buttons::empty(),
            mic: false,
        })
    }
}
