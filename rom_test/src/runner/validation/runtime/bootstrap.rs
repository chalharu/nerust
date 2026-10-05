use nerust_nes_factory::NesFactory;

use super::ValidationRuntime;
use crate::{
    error::RomTestError,
    events::Buttons,
    factory_adapter,
    manifest::RomCase,
    media::{HashingMixer, validation_screen_buffer},
};

impl ValidationRuntime {
    pub(in crate::runner::validation) fn new(
        case: &RomCase,
        rom_bytes: &[u8],
    ) -> Result<Self, RomTestError> {
        let system = factory_adapter::open_nes_system(
            &NesFactory,
            &case.id,
            rom_bytes,
            case.core_options(),
            case.audio_sample_rate(),
        )?;

        Ok(Self {
            screen_buffer: validation_screen_buffer(),
            system,
            mixer: HashingMixer::new(case.audio_sample_rate()),
            audio_sink: Vec::new(),
            frame_counter: 0,
            pad1: Buttons::empty(),
            pad2: Buttons::empty(),
            mic: false,
        })
    }
}
