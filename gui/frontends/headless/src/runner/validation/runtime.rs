mod bootstrap;
mod controller;
mod execution;
mod inspection;

use crate::{factory_adapter::TestSystem, media::HashingMixer};

pub(super) struct ValidationRuntime {
    system: TestSystem,
    mixer: HashingMixer,
    serial: Vec<u8>,
    frame_counter: u64,
}
