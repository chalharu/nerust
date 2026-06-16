mod bootstrap;
mod controller;
mod execution;
mod inspection;

use crate::media::HashingMixer;
use nerust_input_nes::frame::Buttons;
use nerust_input_nes_runtime::nes_input_cell::{NesInputCell, SharedNesInputCell};
use nerust_nes_core::Core;
use nerust_nes_device::nes_pad::NesPadDevice;
use nerust_screen_buffer::screen_buffer::ScreenBuffer;
use std::sync::Arc;

pub(super) struct ValidationRuntime {
    screen_buffer: ScreenBuffer,
    core: Core,
    mixer: HashingMixer,
    controller: NesPadDevice<SharedNesInputCell>,
    cell: Arc<NesInputCell>,
    frame_counter: u64,
    pad1: Buttons,
    pad2: Buttons,
    mic: bool,
}
