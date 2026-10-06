mod bootstrap;
mod controller;
mod execution;
mod inspection;

use crate::{factory_adapter::TestSystem, media::HashingMixer};

pub(super) struct ValidationRuntime {
    system: TestSystem,
    mixer: HashingMixer,
    frame_counter: u64,
}
