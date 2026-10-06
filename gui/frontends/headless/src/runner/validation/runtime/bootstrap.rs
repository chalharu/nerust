use nerust_core_traits::factory::CoreFactory;

use super::ValidationRuntime;
use crate::{error::RomTestError, factory_adapter, manifest::RomCase, media::HashingMixer};

impl ValidationRuntime {
    pub(in crate::runner::validation) fn new(
        factory: &dyn CoreFactory,
        case: &RomCase,
        rom_bytes: &[u8],
    ) -> Result<Self, RomTestError> {
        let system = factory_adapter::open_headless_system(
            factory,
            &case.id,
            rom_bytes,
            case.options.clone(),
            case.audio_sample_rate(),
        )?;

        Ok(Self {
            system,
            mixer: HashingMixer::new(case.audio_sample_rate()),
            frame_counter: 0,
        })
    }
}
