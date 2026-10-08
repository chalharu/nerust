mod bootstrap;
mod controller;
mod execution;

use std::collections::HashMap;

use nerust_core_traits::audio::AudioBackend;

use crate::{factory_adapter::TestSystem, media::HashingMixer};

pub(super) struct ValidationRuntime {
    system: TestSystem,
    mixer: HashingMixer,
    serial: HashMap<String, Vec<u8>>,
    frame_counter: u64,
}

impl ValidationRuntime {
    pub(in crate::runner::validation) fn system_name(&self) -> &'static str {
        self.system.system_name()
    }

    /// Capability views handed to artifact recording: recording code
    /// receives only the role it needs, never the runtime, so it
    /// provably cannot step, reset, or drive input.
    pub(in crate::runner::validation) fn observe(
        &mut self,
    ) -> crate::factory_adapter::FrameObserve<'_> {
        self.system.observe()
    }

    pub(in crate::runner::validation) fn inspect(
        &self,
    ) -> crate::factory_adapter::SystemInspector<'_> {
        self.system.inspect()
    }

    pub(in crate::runner::validation) fn serial_logs(&self) -> &HashMap<String, Vec<u8>> {
        &self.serial
    }

    pub(in crate::runner::validation) fn audio_snapshot(&self) -> super::artifacts::AudioSnapshot {
        super::artifacts::AudioSnapshot {
            sample_rate: self.mixer.sample_rate(),
            samples: self.mixer.samples(),
            hash: self.mixer.checksum(),
        }
    }
}
