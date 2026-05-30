use crate::ConsoleMetrics;
use std::sync::{Arc, RwLock};

#[derive(Debug, Clone)]
pub(crate) struct SharedConsoleMetrics {
    inner: Arc<RwLock<ConsoleMetrics>>,
}

impl SharedConsoleMetrics {
    pub(crate) fn new(initial: ConsoleMetrics) -> Self {
        Self {
            inner: Arc::new(RwLock::new(initial)),
        }
    }

    pub(crate) fn snapshot(&self) -> ConsoleMetrics {
        *self.inner.read().unwrap_or_else(|err| err.into_inner())
    }

    pub(super) fn publish(
        &self,
        frame_counter: u64,
        paused: bool,
        loaded: bool,
        emulation_fps: f32,
        speed_multiplier: f32,
    ) {
        let mut metrics = self.inner.write().unwrap_or_else(|err| err.into_inner());
        *metrics = ConsoleMetrics {
            frame_counter,
            emulation_fps,
            speed_multiplier,
            loaded,
            paused,
            ..ConsoleMetrics::default()
        };
    }
}
