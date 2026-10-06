mod bootstrap;
mod controller;
mod execution;
mod inspection;

use crate::{events::Buttons, factory_adapter::TestSystem, media::HashingMixer};

pub(super) struct ValidationRuntime {
    system: TestSystem,
    mixer: HashingMixer,
    frame_counter: u64,
    pad1: Buttons,
    pad2: Buttons,
    mic: bool,
}
