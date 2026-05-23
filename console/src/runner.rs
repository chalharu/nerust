mod data;
mod persistence;
mod runtime;

pub(crate) use data::ConsoleData;

use super::ConsoleMetrics;
use crate::core_api::StandardController;
use crate::screen_api::ScreenBuffer;
use nerust_timer::{TARGET_FPS, Timer};
use std::sync::mpsc::Receiver;
use std::sync::{Arc, RwLock};

pub(super) struct ConsoleRunner {
    timer: Timer,
    controller: StandardController,
    paused: bool,
    frame_counter: u64,

    stop_receiver: Receiver<()>,
    data_receiver: Receiver<ConsoleData>,
    screen: ScreenBuffer,
    frame_buffer: Arc<RwLock<Box<[u8]>>>,
    metrics: Arc<RwLock<ConsoleMetrics>>,
}

impl ConsoleRunner {
    pub(super) fn new(
        data_receiver: Receiver<ConsoleData>,
        stop_receiver: Receiver<()>,
        screen: ScreenBuffer,
        frame_buffer: Arc<RwLock<Box<[u8]>>>,
        metrics: Arc<RwLock<ConsoleMetrics>>,
    ) -> Self {
        Self {
            data_receiver,
            stop_receiver,

            timer: Timer::new(),
            controller: StandardController::new(),
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
        self.screen.copy_frame_buffer(frame_buffer.as_mut());
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
        let mut metrics = self.metrics.write().unwrap_or_else(|err| err.into_inner());
        *metrics = ConsoleMetrics {
            frame_counter: self.frame_counter,
            emulation_fps,
            speed_multiplier,
            loaded,
            paused: self.paused,
        };
    }
}
