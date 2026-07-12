mod bootstrap;
mod controller;
mod execution;
mod inspection;

use nerust_input_traits::ControllerCollection;
use nerust_nes_core::Core;
use nerust_render_base::FrameBuffer;

use crate::{events::Buttons, media::HashingMixer};

pub(super) struct ValidationRuntime {
    screen_buffer: FrameBuffer,
    core: Core,
    controller: ControllerCollection,
    mixer: HashingMixer,
    frame_counter: u64,
    pad1: Buttons,
    pad2: Buttons,
    mic: bool,
}
