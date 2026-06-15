use std::sync::Arc;

use super::ValidationRuntime;
use crate::error::RomTestError;
use crate::manifest::RomCase;
use crate::media::{HashingMixer, validation_screen_buffer};
use nerust_cartridge_data::parse_cartridge_bytes;
use nerust_contract_core::input::InputCell;
use nerust_input_nes::frame::Buttons;
use nerust_input_nes_runtime::nes_pad_device::NesPadDevice;
use nerust_nes_core::Core;

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

        let cell = Arc::new(InputCell::new());
        Ok(Self {
            screen_buffer: validation_screen_buffer(),
            core,
            controller: NesPadDevice::new(cell.clone()),
            cell,
            mixer: HashingMixer::new(case.audio_sample_rate()),
            frame_counter: 0,
            pad1: Buttons::empty(),
            pad2: Buttons::empty(),
        })
    }
}
