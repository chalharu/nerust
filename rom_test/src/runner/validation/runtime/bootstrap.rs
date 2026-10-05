use nerust_core_traits::factory::CoreFactory;

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
        factory: &dyn CoreFactory,
        case: &RomCase,
        rom_bytes: &[u8],
    ) -> Result<Self, RomTestError> {
        let system = factory_adapter::open_nes_system(
            factory,
            &case.id,
            rom_bytes,
            case.mmc3_irq_variant,
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
