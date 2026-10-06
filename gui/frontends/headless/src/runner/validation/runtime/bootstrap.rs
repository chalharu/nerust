use super::ValidationRuntime;
use crate::{error::RomTestError, factory_adapter, manifest::RomCase, media::HashingMixer};

impl ValidationRuntime {
    pub(in crate::runner::validation) fn new(
        factories: &[Box<dyn nerust_core_traits::factory::CoreFactory>],
        case: &RomCase,
        rom_bytes: &[u8],
    ) -> Result<Self, RomTestError> {
        let system = factory_adapter::open_headless_system(
            factories,
            &case.id,
            rom_bytes,
            case.options.clone(),
            case.audio_sample_rate(),
        )?;

        Ok(Self {
            system,
            mixer: HashingMixer::new(case.audio_sample_rate()),
            serial: Vec::new(),
            frame_counter: 0,
        })
    }
}
