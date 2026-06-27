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

use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
    mpsc::{SyncSender, TrySendError, sync_channel},
};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use nerust_contract_core::audio::{AudioBackend, AudioBackendFactory};

/// CPAL-based audio backend.
///
/// Implements [`AudioBackend`] for use with any consumer that accepts the trait.
pub struct CpalAudio {
    stream: cpal::Stream,
    data_sender: SyncSender<f32>,
    playing: Arc<AtomicBool>,
    needs_clear: Arc<AtomicBool>,
    sample_rate: u32,
}

impl CpalAudio {
    /// Create a `CpalAudio` backend.
    ///
    /// * `sample_rate` – playback rate requested by the core.
    /// * `latency_ms` – target latency in milliseconds.
    ///
    /// Returns `Err` (with a descriptive message) if no audio device or stream
    /// can be opened.
    pub fn new(sample_rate: u32, latency_ms: u16) -> Result<Self, String> {
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
        let device_sample_rate = supported_config.sample_rate();

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
            "cpal audio: device='{device_name}' requested_rate={sample_rate} device_rate={device_sample_rate} channels={channels}",
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
            sample_rate,
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
        match self.data_sender.try_send(data) {
            Ok(()) | Err(TrySendError::Full(_)) => {}
            Err(TrySendError::Disconnected(_)) => {
                log::warn!("cpal audio: channel send failed (receiver dropped)");
            }
        }
    }

    fn sample_rate(&self) -> u32 {
        self.sample_rate
    }
}

/// Factory for creating and probing CPAL audio backends.
pub struct CpalFactory;

impl AudioBackendFactory for CpalFactory {
    fn name(&self) -> &'static str {
        "CPAL"
    }

    fn probe(&self) -> Vec<u32> {
        const COMMON: [u32; 6] = [22_050, 24_000, 44_100, 48_000, 88_200, 96_000];
        #[cfg(not(target_os = "android"))]
        {
            // Desktop: trust supported_output_configs() range matching.
            let device = match cpal::default_host().default_output_device() {
                Some(d) => d,
                None => return vec![],
            };
            let configs: Vec<_> = match device.supported_output_configs() {
                Ok(c) => c.collect(),
                Err(_) => return vec![],
            };
            COMMON
                .iter()
                .copied()
                .filter(|&rate| {
                    configs
                        .iter()
                        .any(|cfg| rate >= cfg.min_sample_rate() && rate <= cfg.max_sample_rate())
                })
                .collect()
        }
        #[cfg(target_os = "android")]
        {
            // Android (OpenSL ES) returns a single wide range (e.g. 8000-48000)
            // that includes rates which don't actually work.
            // Verify by creating a short-lived backend for each candidate.
            COMMON
                .iter()
                .copied()
                .filter(|&rate| CpalAudio::new(rate, 10).is_ok())
                .collect()
        }
    }

    fn build(&self, sample_rate: u32, latency_ms: u32) -> Option<Box<dyn AudioBackend>> {
        let latency = u16::try_from(latency_ms).unwrap_or(u16::MAX);
        CpalAudio::new(sample_rate, latency)
            .ok()
            .map(|a| Box::new(a) as Box<dyn AudioBackend>)
    }
}

/// Static singleton for use with [`AudioBackendRegistry`](nerust_contract_core::audio::AudioBackendRegistry).
pub static CPAL: CpalFactory = CpalFactory;
