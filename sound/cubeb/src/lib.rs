use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
    mpsc::{SyncSender, TrySendError, sync_channel},
};

use cubeb::{MonoFrame, SampleFormat, StreamParamsBuilder};
use log::{info, warn};
use nerust_core_traits::audio::{AudioBackend, AudioBackendFactory};

pub struct CubebAudio {
    stream: cubeb::Stream<MonoFrame<f32>>,
    data_sender: SyncSender<f32>,
    playing: Arc<AtomicBool>,
    sample_rate: u32,
}

// SAFETY: cubeb::Stream wraps a C API handle that is safe to send between
// threads. Stream operations (start/stop) are thread-safe in cubeb.
unsafe impl Send for CubebAudio {}

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

        let (sender, receiver) = sync_channel::<f32>(sample_rate as usize);

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
                    for frame in output.iter_mut() {
                        frame.m = receiver.try_recv().unwrap_or(0.0);
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
            stream,
            data_sender: sender,
            playing,
            sample_rate,
        })
    }
}

impl AudioBackend for CubebAudio {
    fn start(&mut self) {
        self.playing.store(true, Ordering::Relaxed);
        if let Err(e) = self.stream.start() {
            warn!("cubeb stream start failed: {e}");
        }
    }

    fn pause(&mut self) {
        self.playing.store(false, Ordering::Relaxed);
        if let Err(e) = self.stream.stop() {
            warn!("cubeb stream stop failed: {e}");
        }
    }

    fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    fn push(&mut self, sample: f32) {
        if let Err(TrySendError::Full(_)) = self.data_sender.try_send(sample) {
            // buffer full; sample dropped
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

pub static CUBEB: CubebFactory = CubebFactory;
