mod bootstrap;
mod controller;
mod execution;
mod inspection;

use std::sync::Arc;

use nerust_nes_controller::nes_input_cell::{NesInputCell, SharedNesInputCell};
use nerust_nes_core::{Core, input_types::Buttons};
use nerust_nes_device::nes_pad::NesPadDevice;
use nerust_render_base::{FrameBuffer, PixelFormat};

use crate::media::HashingMixer;

pub(super) struct ValidationRuntime {
    screen_buffer: FrameBuffer,
    core: Core,
    mixer: HashingMixer,
    controller: NesPadDevice<SharedNesInputCell>,
    cell: Arc<NesInputCell>,
    frame_counter: u64,
    pad1: Buttons,
    pad2: Buttons,
    mic: bool,
}
