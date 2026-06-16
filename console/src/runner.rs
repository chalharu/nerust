pub(crate) mod data;
pub(crate) mod metrics;
mod persistence;
mod runtime;

use self::metrics::SharedConsoleMetrics;
use data::ConsoleData;
use nerust_input_nes_runtime::ControllerState;
use nerust_screen_buffer::screen_buffer::ScreenBuffer;
use nerust_screen_video::FrameBuffer;
use nerust_timer::{TARGET_FPS, Timer};
use std::sync::atomic::AtomicBool;
use std::sync::mpsc::Receiver;
use std::sync::{Arc, Mutex};

pub(super) struct ConsoleRunner {
    timer: Timer,
    controller: Box<dyn ControllerState>,
    paused: bool,
    frame_counter: u64,
    frame_buffer_updated: Arc<AtomicBool>,
    stop_receiver: Receiver<()>,
    data_receiver: Receiver<ConsoleData>,
    screen: ScreenBuffer,
    frame_buffer: Arc<Mutex<FrameBuffer>>,
    screen_backing: FrameBuffer,
    metrics: SharedConsoleMetrics,
}

impl ConsoleRunner {
    pub(super) fn new(
        data_receiver: Receiver<ConsoleData>,
        stop_receiver: Receiver<()>,
        screen: ScreenBuffer,
        frame_buffer: Arc<Mutex<FrameBuffer>>,
        frame_buffer_updated: Arc<AtomicBool>,
        screen_backing: FrameBuffer,
        metrics: SharedConsoleMetrics,
        controller: Box<dyn ControllerState>,
    ) -> Self {
        Self {
            data_receiver,
            stop_receiver,
            frame_buffer_updated,
            timer: Timer::new(),
            controller,
            paused: true,
            frame_counter: 0,
            screen,
            frame_buffer,
            screen_backing,
            metrics,
        }
    }

    fn publish_frame(&mut self) {
        self.screen.write_frame_into(self.screen_backing.as_mut());
        let mut guard = self.frame_buffer.lock().unwrap();
        std::mem::swap(&mut *guard, &mut self.screen_backing);
        self.frame_buffer_updated
            .store(true, std::sync::atomic::Ordering::Release);
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
