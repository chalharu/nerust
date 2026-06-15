//! CPAL-based audio backend for desktop and mobile targets.
//!
//! `CpalAudio` wraps a CPAL output stream and implements the `AudioBackend` trait
//! (and the legacy `MixerInput` / `Sound` traits for backwards compatibility).
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
use nerust_sound_traits::{MixerInput, Sound};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{SyncSender, TrySendError, sync_channel};

/// Multiplier cap on the internal oversampling rate relative to device rate.
const OVERSAMPLE_FACTOR: u32 = 4;

/// CPAL-based audio backend.
///
/// Implements [`AudioBackend`], [`Sound`], and [`MixerInput`] so it can be used
/// as a drop-in replacement for the OpenAL backend on any platform CPAL supports.
pub struct CpalAudio {
    /// CPAL stream – must be kept alive for audio to continue playing.
    stream: cpal::Stream,
    /// Sends f32 samples to the CPAL callback.
    data_sender: SyncSender<f32>,
    /// Desired playback state shared with the audio callback.
    playing: Arc<AtomicBool>,
    /// Effective source sample rate returned to the NES core.
    source_sample_rate: u32,
}

impl CpalAudio {
    /// Create a `CpalAudio` backend.
    ///
    /// * `sample_rate` – playback rate requested by the core.
    /// * `output_rate` – the NES CPU clock rate (used as the pre-resampler
    ///   source rate cap).
    ///
    /// Returns `Err` (with a descriptive message) if no audio device or stream
    /// can be opened.
    pub fn new(sample_rate: u32, output_rate: u32) -> Result<Self, String> {
        let host = cpal::default_host();

        let device = host
            .default_output_device()
            .ok_or_else(|| "no default audio output device available".to_string())?;

        let supported_config = device
            .default_output_config()
            .map_err(|e| format!("failed to query default audio output config: {e}"))?;

        let channels = supported_config.channels();
        let playing = Arc::new(AtomicBool::new(false));

        let source_sample_rate = output_rate
            .min(sample_rate.saturating_mul(OVERSAMPLE_FACTOR))
            .max(sample_rate);

        let mut stream_config = supported_config.config();
        stream_config.sample_rate = sample_rate;
        let queue_capacity =
            usize::try_from((sample_rate / 10).max(1)).expect("queue capacity fits into usize");
        let (data_sender, data_receiver) = sync_channel::<f32>(queue_capacity);
        let callback_playing = playing.clone();

        let device_name = device.description()
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
            source_sample_rate,
        })
    }
}

impl Sound for CpalAudio {
    fn start(&mut self) {
        self.playing.store(true, Ordering::Release);
        if let Err(e) = self.stream.play() {
            log::error!("failed to start cpal audio stream: {e}");
        }
    }

    fn pause(&mut self) {
        self.playing.store(false, Ordering::Release);
        if let Err(e) = self.stream.pause() {
            log::warn!("failed to pause cpal audio stream: {e}");
        }
    }
}

impl MixerInput for CpalAudio {
    fn push(&mut self, data: f32) {
        match self.data_sender.try_send(data) {
            Ok(()) | Err(TrySendError::Full(_)) => {}
            Err(TrySendError::Disconnected(_)) => {
                log::warn!("cpal audio: channel send failed (receiver dropped)");
            }
        }
    }

    fn sample_rate(&self) -> u32 {
        self.source_sample_rate
    }
}

impl AudioBackend for CpalAudio {
    fn start(&mut self) {
        Sound::start(self);
    }

    fn pause(&mut self) {
        Sound::pause(self);
    }

    fn push(&mut self, data: f32) {
        MixerInput::push(self, data);
    }

    fn sample_rate(&self) -> u32 {
        MixerInput::sample_rate(self)
    }
}
