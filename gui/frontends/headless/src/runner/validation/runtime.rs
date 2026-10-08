mod bootstrap;
mod controller;
mod execution;
mod inspection;

use std::collections::HashMap;

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
}
