use nerust_nes_core::{Core, input_types::Buttons, rom_parse};
use nerust_nes_device::famicom_set::FamicomSet;

use super::ValidationRuntime;
use crate::{
    error::RomTestError,
    manifest::RomCase,
    media::{HashingMixer, validation_screen_buffer},
};

impl ValidationRuntime {
    pub(in crate::runner::validation) fn new(
        case: &RomCase,
        rom_bytes: &[u8],
    ) -> Result<Self, RomTestError> {
        let cartridge_data =
            rom_parse::parse_rom(rom_bytes).map_err(|error| RomTestError::CoreConstruction {
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

        Ok(Self {
            screen_buffer: validation_screen_buffer(),
            core,
            controller: FamicomSet::new(),
            mixer: HashingMixer::new(case.audio_sample_rate()),
            frame_counter: 0,
            pad1: Buttons::empty(),
            pad2: Buttons::empty(),
            mic: false,
        })
    }
}
