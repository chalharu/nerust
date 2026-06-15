pub(crate) mod data;
pub(crate) mod metrics;
mod persistence;
mod runtime;

use self::metrics::SharedConsoleMetrics;
use data::ConsoleData;
use nerust_input_nes_runtime::ControllerState;
use nerust_screen_buffer::screen_buffer::ScreenBuffer;
use nerust_timer::{TARGET_FPS, Timer};
use std::sync::mpsc::Receiver;
use std::sync::{Arc, RwLock};

pub(super) struct ConsoleRunner {
    timer: Timer,
    controller: Box<dyn ControllerState>,
    paused: bool,
    frame_counter: u64,

    stop_receiver: Receiver<()>,
    data_receiver: Receiver<ConsoleData>,
    screen: ScreenBuffer,
    frame_buffer: Arc<RwLock<Box<[u8]>>>,
    metrics: SharedConsoleMetrics,
}

impl ConsoleRunner {
    pub(super) fn new(
        data_receiver: Receiver<ConsoleData>,
        stop_receiver: Receiver<()>,
        screen: ScreenBuffer,
        frame_buffer: Arc<RwLock<Box<[u8]>>>,
        metrics: SharedConsoleMetrics,
        controller: Box<dyn ControllerState>,
    ) -> Self {
        Self {
            data_receiver,
            stop_receiver,

            timer: Timer::new(),
            controller,
            paused: true,
            frame_counter: 0,
            screen,
            frame_buffer,
            metrics,
        }
    }

    fn publish_frame(&self) {
        let mut frame_buffer = self
            .frame_buffer
            .write()
            .unwrap_or_else(|err| err.into_inner());
        self.screen.write_frame_into(frame_buffer.as_mut());
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
