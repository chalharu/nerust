mod bootstrap;
mod controller;
mod execution;
mod inspection;

use std::sync::Arc;

use crate::media::HashingMixer;
use nerust_contract_core::input::InputCell;
use nerust_input_nes::frame::Buttons;
use nerust_input_nes_runtime::nes_pad_device::NesPadDevice;
use nerust_nes_core::Core;
use nerust_screen_buffer::screen_buffer::ScreenBuffer;

pub(super) struct ValidationRuntime {
    screen_buffer: ScreenBuffer,
    core: Core,
    mixer: HashingMixer,
    controller: NesPadDevice<Arc<InputCell<2>>>,
    cell: Arc<InputCell<2>>,
    frame_counter: u64,
    pad1: Buttons,
    pad2: Buttons,
}
