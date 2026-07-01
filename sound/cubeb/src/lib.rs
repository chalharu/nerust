use std::{
    mem::ManuallyDrop,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

use cubeb::{MonoFrame, SampleFormat, StreamParamsBuilder};
use flume::{Sender, TrySendError, bounded};
use log::{info, warn};
use nerust_core_traits::audio::{AudioBackend, AudioBackendFactory};

pub struct CubebAudio {
    stream: ManuallyDrop<cubeb::Stream<MonoFrame<f32>>>,
    data_sender: Sender<f32>,
    playing: Arc<AtomicBool>,
    sample_rate: u32,
    ctx: ManuallyDrop<cubeb::Context>,
}

// SAFETY: cubeb::Stream wraps a C API handle that is safe to send between
// threads. Stream operations (start/stop) are thread-safe in cubeb.
unsafe impl Send for CubebAudio {}

impl Drop for CubebAudio {
    fn drop(&mut self) {
        // Stop the stream before dropping it to avoid potential issues.
        if let Err(e) = self.stream.stop() {
            warn!("cubeb stream stop failed during drop: {e}");
        }
        // Manually drop the stream and context to ensure proper cleanup.
        unsafe {
            ManuallyDrop::drop(&mut self.stream);
            ManuallyDrop::drop(&mut self.ctx);
        }
    }
}

impl CubebAudio {
    pub fn new(sample_rate: u32, latency_ms: u32) -> Result<Self, String> {
        let ctx = cubeb::init("nerust").map_err(|e| format!("cubeb init failed: {e}"))?;

        let latency_frames = (sample_rate as u64 * latency_ms as u64 / 1000) as u32;

        let params = StreamParamsBuilder::new()
            .format(SampleFormat::Float32LE)
            .rate(sample_rate)
            .channels(1)
            .layout(cubeb::ChannelLayout::UNDEFINED)
            .prefs(cubeb::StreamPrefs::NONE)
            .take();

        let playing = Arc::new(AtomicBool::new(true));
        let playing_clone = playing.clone();

        let (sender, receiver) = bounded::<f32>(sample_rate as usize);

        let mut builder = cubeb::StreamBuilder::<MonoFrame<f32>>::new();
        builder
            .name("output")
            .default_output(&params)
            .latency(latency_frames)
            .data_callback(
                move |_input: &[MonoFrame<f32>], output: &mut [MonoFrame<f32>]| -> isize {
                    if !playing_clone.load(Ordering::Relaxed) {
                        return 0;
                    }
                    let mut last = 0.0f32;
                    for frame in output.iter_mut() {
                        if let Ok(sample) = receiver.try_recv() {
                            last = sample;
                        }
                        frame.m = last;
                    }
                    output.len() as isize
                },
            )
            .state_callback(|_state: cubeb::State| {});

        let stream = builder
            .init(&ctx)
            .map_err(|e| format!("cubeb stream init failed: {e}"))?;

        info!(
            "cubeb: created stream at {} Hz ({} frames latency)",
            sample_rate, latency_frames
        );

        Ok(Self {
            stream: ManuallyDrop::new(stream),
            data_sender: sender,
            playing,
            sample_rate,
            ctx: ManuallyDrop::new(ctx),
        })
    }
}

impl AudioBackend for CubebAudio {
    fn start(&mut self) {
        // &mut selfによる排他制御があるので、start/stopの呼び出しはスレッドセーフである。
        if let Err(e) = self.stream.start() {
            warn!("cubeb stream start failed: {e}");
        }
        self.playing.store(true, Ordering::Release);
    }

    fn pause(&mut self) {
        // &mut selfによる排他制御があるので、start/stopの呼び出しはスレッドセーフである。
        if let Err(e) = self.stream.stop() {
            warn!("cubeb stream stop failed: {e}");
        }
        self.playing.store(false, Ordering::Release);
    }

    fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    fn push(&mut self, sample: f32) {
        match self.data_sender.try_send(sample) {
            Ok(()) | Err(TrySendError::Full(_)) => {}
            Err(TrySendError::Disconnected(_)) => {
                log::warn!("cubeb audio: channel send failed (receiver dropped)");
            }
        }
    }
}

pub struct CubebFactory;

impl AudioBackendFactory for CubebFactory {
    fn name(&self) -> &'static str {
        "cubeb"
    }

    fn probe(&self) -> Vec<u32> {
        vec![44_100, 48_000]
    }

    fn build(&self, sample_rate: u32, latency_ms: u32) -> Option<Box<dyn AudioBackend>> {
        CubebAudio::new(sample_rate, latency_ms)
            .inspect_err(|e| warn!("cubeb backend build failed: {e}"))
            .ok()
            .map(|a| Box::new(a) as Box<dyn AudioBackend>)
    }
}
