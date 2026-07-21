use std::collections::HashMap;

use nerust_core_traits::{ConsoleCore as _, CoreConfig, DynCoreOptions};
use nerust_input_traits::ControllerCollection;
use nerust_nes_core::console_core::NesConsoleCore;
use nerust_nes_device::famicom_set::{FamicomPadP1, FamicomPadP2};

use super::ValidationRuntime;
use crate::{
    error::RomTestError,
    events::Buttons,
    manifest::RomCase,
    media::{HashingMixer, validation_screen_buffer},
};
use nerust_nes_core::debugger::nes::NesDebugger;

impl ValidationRuntime {
    pub(in crate::runner::validation) fn new(
        case: &RomCase,
        rom_bytes: &[u8],
    ) -> Result<Self, RomTestError> {
        let mut console_core = NesConsoleCore::new_minimal();
        let opts: Box<dyn DynCoreOptions> = case.core_options().into();
        let config = CoreConfig {
            region: None,
            bios_paths: HashMap::new(),
            controllers: HashMap::new(),
            core_options: Some(opts),
        };
        console_core
            .load(rom_bytes, &config)
            .map_err(|error| RomTestError::CoreConstruction {
                case_id: case.id.clone(),
                message: error.to_string(),
            })?;

        let debugger =
            console_core
                .create_debugger()
                .ok_or_else(|| RomTestError::CoreConstruction {
                    case_id: case.id.clone(),
                    message: "debugger not supported".to_string(),
                })?;
        let debugger =
            debugger
                .downcast::<NesDebugger>()
                .map_err(|_| RomTestError::CoreConstruction {
                    case_id: case.id.clone(),
                    message: "expected NES debugger".to_string(),
                })?;

        Ok(Self {
            screen_buffer: validation_screen_buffer(),
            core: Box::new(console_core),
            debugger,
            controller: ControllerCollection::new(vec![
                Box::new(FamicomPadP1::new()),
                Box::new(FamicomPadP2::new()),
            ]),
            mixer: HashingMixer::new(case.audio_sample_rate()),
            frame_counter: 0,
            pad1: Buttons::empty(),
            pad2: Buttons::empty(),
            mic: false,
        })
    }
}
