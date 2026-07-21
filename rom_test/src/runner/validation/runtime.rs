mod bootstrap;
mod controller;
mod execution;
mod inspection;

use nerust_core_traits::ConsoleCore;
use nerust_input_traits::ControllerCollection;
use nerust_nes_core::debugger::nes::NesDebugger;
use nerust_render_traits::FrameBuffer;

use crate::{events::Buttons, media::HashingMixer};

pub(super) struct ValidationRuntime {
    screen_buffer: FrameBuffer,
    core: Box<dyn ConsoleCore>,
    debugger: Box<NesDebugger>,
    controller: ControllerCollection,
    mixer: HashingMixer,
    frame_counter: u64,
    pad1: Buttons,
    pad2: Buttons,
    mic: bool,
}
