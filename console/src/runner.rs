pub(crate) mod data;
pub(crate) mod metrics;
mod persistence;
mod runtime;

use self::metrics::SharedConsoleMetrics;
use data::ConsoleData;
use nerust_contract_core::GpuCommand;
use nerust_contract_core::GpuCommandList;
use nerust_contract_core::audio::AudioBackend;
use nerust_contract_core::channel::FrameChannelConsole;
use nerust_input_nes_runtime::ControllerState;
use nerust_screen_video::FrameBuffer;
use nerust_timer::{TARGET_FPS, Timer};
use std::sync::mpsc::Receiver;
use std::sync::{Arc, Mutex};

pub(super) struct ConsoleRunner {
    timer: Timer,
    controller: Box<dyn ControllerState>,
    paused: bool,
    frame_counter: u64,
    channel: FrameChannelConsole,
    stop_receiver: Receiver<()>,
    data_receiver: Receiver<ConsoleData>,
    ppu_fb: FrameBuffer,
    frame_buffer: Arc<Mutex<FrameBuffer>>,
    metrics: SharedConsoleMetrics,
    audio: Box<dyn AudioBackend>,
}

impl ConsoleRunner {
    #[allow(clippy::too_many_arguments)]
    pub(super) fn new(
        data_receiver: Receiver<ConsoleData>,
        stop_receiver: Receiver<()>,
        ppu_fb: FrameBuffer,
        frame_buffer: Arc<Mutex<FrameBuffer>>,
        channel: FrameChannelConsole,
        metrics: SharedConsoleMetrics,
        controller: Box<dyn ControllerState>,
        audio: Box<dyn AudioBackend>,
    ) -> Self {
        Self {
            data_receiver,
            stop_receiver,
            timer: Timer::new(),
            controller,
            paused: true,
            frame_counter: 0,
            channel,
            ppu_fb,
            frame_buffer,
            metrics,
            audio,
        }
    }

    fn publish_frame(&mut self) {
        // PPU が ppu_fb に書き込んだデータを shared と swap して publish
        // state export 時は shared から直接読む (ppu_fb は swap で上書きされる)
        if self.channel.try_send_frame(GpuCommandList {
            commands: vec![GpuCommand::Blit { slot: 0 }],
        }) {
            let mut guard = self.frame_buffer.lock().unwrap();
            std::mem::swap(&mut *guard, &mut self.ppu_fb);
        }
    }

    fn publish_metrics(&self, loaded: bool) {
        let emulation_fps = if loaded && !self.paused {
            self.timer.as_fps()
        } else {
            0.0
        };
        let speed_multiplier = if emulation_fps > 0.0 {
            emulation_fps / TARGET_FPS
        } else {
            0.0
        };
        self.metrics.publish(
            self.frame_counter,
            self.paused,
            loaded,
            emulation_fps,
            speed_multiplier,
        );
    }
}
