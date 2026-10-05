mod bootstrap;
mod controller;
mod execution;
mod inspection;

use nerust_core_traits::audio::StereoSample;
use nerust_render_traits::FrameBuffer;

use crate::{events::Buttons, factory_adapter::TestSystem, media::HashingMixer};

pub(super) struct ValidationRuntime {
    screen_buffer: FrameBuffer,
    system: TestSystem,
    mixer: HashingMixer,
    audio_sink: Vec<StereoSample>,
    frame_counter: u64,
    pad1: Buttons,
    pad2: Buttons,
    mic: bool,
}
