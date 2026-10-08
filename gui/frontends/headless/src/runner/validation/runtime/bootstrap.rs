use super::ValidationRuntime;
use crate::{error::RomTestError, factory_adapter, manifest::RomCase, media::HashingMixer};

impl ValidationRuntime {
    pub(in crate::runner::validation) fn new(
        factories: &[Box<dyn nerust_core_traits::factory::CoreFactory>],
        case: &RomCase,
        rom_bytes: &[u8],
    ) -> Result<Self, RomTestError> {
        let mut system = factory_adapter::open_case_system(factories, case, rom_bytes)?;
        // Pre-register every open-time channel as a cumulative-log
        // key: the set is fixed at open, so per-frame accumulation
        // uses `get_mut` with zero key clones instead of `entry`.
        let serial = system
            .observe()
            .channel_names()
            .iter()
            .map(|channel| (channel.clone(), Vec::new()))
            .collect();

        Ok(Self {
            system,
            mixer: HashingMixer::new(case.audio_sample_rate()),
            serial,
            frame_counter: 0,
        })
    }
}
