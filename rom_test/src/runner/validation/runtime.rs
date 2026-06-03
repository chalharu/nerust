mod bootstrap;
mod controller;
mod execution;
mod inspection;

use crate::media::HashingMixer;
use nerust_core::Core;
use nerust_input_nes::frame::Buttons;
use nerust_input_nes_runtime::StandardController;
use nerust_screen_buffer::screen_buffer::ScreenBuffer;

pub(super) struct ValidationRuntime {
    screen_buffer: ScreenBuffer,
    core: Core,
    mixer: HashingMixer,
    controller: StandardController,
    frame_counter: u64,
    pad1: Buttons,
    pad2: Buttons,
}
