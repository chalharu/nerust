//! CPAL-based audio backend for desktop and mobile targets.
//!
//! `CpalAudio` wraps a CPAL output stream and implements the [`AudioBackend`] trait.
//!
//! # Lifecycle
//!
//! * Call [`CpalAudio::new`] at startup.  Backend creation is **fallible** – do
//!   not silently fall back if it fails; propagate the error so the caller can
//!   select a different backend or surface the problem.
//! * Call [`AudioBackend::start`] / [`AudioBackend::pause`] to mirror the app
//!   lifecycle (foreground / background).
//! * Feed samples via [`AudioBackend::push`]; the NES APU calls this at the rate
//!   returned by [`AudioBackend::sample_rate`].

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use nerust_contract_core::audio::AudioBackend;
use nerust_soundfilter::resampler::{Resampler, SimpleDownSampler};
use nerust_soundfilter::{Filter, NesFilter};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{SyncSender, TrySendError, sync_channel};

/// Multiplier cap on the internal oversampling rate relative to device rate.
const OVERSAMPLE_FACTOR: u32 = 4;

/// CPAL-based audio backend.
///
/// Implements [`AudioBackend`] for use with any consumer that accepts the trait.
pub struct CpalAudio {
    stream: cpal::Stream,
    data_sender: SyncSender<f32>,
    playing: Arc<AtomicBool>,
    needs_clear: Arc<AtomicBool>,
    filter: NesFilter,
    gain: f32,
    resampler: SimpleDownSampler,
    source_sample_rate: u32,
}

impl CpalAudio {
    /// Create a `CpalAudio` backend.
    ///
    /// * `sample_rate` – playback rate requested by the core.
    /// * `output_rate` – the NES CPU clock rate (used as the pre-resampler
    ///   source rate cap).
    /// * `latency_ms` – target latency in milliseconds.
    /// * `gain` – master volume; `1.0` is full volume, `0.0` is muted.
    ///
    /// Returns `Err` (with a descriptive message) if no audio device or stream
    /// can be opened.
    pub fn new(
        sample_rate: u32,
        output_rate: u32,
        latency_ms: u16,
        gain: f32,
    ) -> Result<Self, String> {
        let host = cpal::default_host();

        let device = host
            .default_output_device()
            .ok_or_else(|| "no default audio output device available".to_string())?;

        let supported_config = device
            .default_output_config()
            .map_err(|e| format!("failed to query default audio output config: {e}"))?;

        let channels = supported_config.channels();
        let playing = Arc::new(AtomicBool::new(false));
        let needs_clear = Arc::new(AtomicBool::new(true));

        let source_sample_rate = output_rate
            .min(sample_rate.saturating_mul(OVERSAMPLE_FACTOR))
            .max(sample_rate);

        let filter = NesFilter::new(sample_rate as f32);
        let resampler =
            SimpleDownSampler::new(f64::from(source_sample_rate), f64::from(sample_rate));

        let requested_frames = (u64::from(sample_rate) * u64::from(latency_ms))
            .div_ceil(1_000)
            .max(1) as u32;
        let queue_capacity = usize::try_from(requested_frames.max(sample_rate / 10))
            .expect("queue capacity fits into usize");
        let (data_sender, data_receiver) = sync_channel::<f32>(queue_capacity);
        let mut stream_config = supported_config.config();
        stream_config.sample_rate = sample_rate;
        match supported_config.buffer_size() {
            cpal::SupportedBufferSize::Range { min, max } => {
                stream_config.buffer_size =
                    cpal::BufferSize::Fixed(requested_frames.clamp(*min, *max));
            }
            cpal::SupportedBufferSize::Unknown => {}
        }
        let callback_playing = playing.clone();
        let callback_needs_clear = needs_clear.clone();

        let device_name = device
            .description()
            .as_ref()
            .map(|d| d.to_string())
            .unwrap_or_else(|_| "<unknown>".to_string());
        log::info!(
            "cpal audio: device='{device_name}' sample_rate={sample_rate} channels={channels}",
        );

        let stream = device
            .build_output_stream(
                &stream_config,
                move |output: &mut [f32], _info: &cpal::OutputCallbackInfo| {
                    if callback_needs_clear.swap(false, Ordering::AcqRel) {
                        while data_receiver.try_recv().is_ok() {}
                    }
                    let active = callback_playing.load(Ordering::Acquire);
                    for frame in output.chunks_mut(channels as usize) {
                        let sample = if active {
                            data_receiver.try_recv().unwrap_or(0.0)
                        } else {
                            0.0
                        };
                        for ch in frame.iter_mut() {
                            *ch = sample;
                        }
                    }
                },
                |err| log::error!("cpal audio stream error: {err}"),
                None,
            )
            .map_err(|e| format!("failed to build cpal audio stream: {e}"))?;

        Ok(Self {
            stream,
            data_sender,
            playing,
            needs_clear,
            filter,
            gain,
            resampler,
            source_sample_rate,
        })
    }
}

impl AudioBackend for CpalAudio {
    fn start(&mut self) {
        self.needs_clear.store(true, Ordering::Release);
        self.playing.store(true, Ordering::Release);
        if let Err(e) = self.stream.play() {
            log::error!("failed to start cpal audio stream: {e}");
        }
    }

    fn pause(&mut self) {
        self.playing.store(false, Ordering::Release);
        self.needs_clear.store(true, Ordering::Release);
        if let Err(e) = self.stream.pause() {
            log::warn!("failed to pause cpal audio stream: {e}");
        }
    }

    fn push(&mut self, data: f32) {
        if let Some(resampled) = self.resampler.step(data) {
            let sample = self.filter.step((resampled * 2.0 - 1.0) * self.gain);
            match self.data_sender.try_send(sample) {
                Ok(()) | Err(TrySendError::Full(_)) => {}
                Err(TrySendError::Disconnected(_)) => {
                    log::warn!("cpal audio: channel send failed (receiver dropped)");
                }
            }
        }
    }

    fn sample_rate(&self) -> u32 {
        self.source_sample_rate
    }
}
