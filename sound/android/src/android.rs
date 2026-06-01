// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

//! CPAL-based `AndroidSound` implementation, compiled only on Android.

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use nerust_sound_traits::{AudioFilterProfile, MixerInput, Sound};
use nerust_soundfilter::resampler::{Resampler, SimpleDownSampler};
use nerust_soundfilter::{Filter, NesFilter, SnesFilter};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{SyncSender, TrySendError, sync_channel};

/// Multiplier cap on the internal oversampling rate relative to device rate.
///
/// Mirrors the `CORE_AUDIO_OVERSAMPLE` constant used in the OpenAL backend.
const OVERSAMPLE_FACTOR: u32 = 4;

/// Android-native audio backend using CPAL (AAudio on API 26+).
///
/// Implements both [`Sound`] and [`MixerInput`] so it can be used as a
/// drop-in replacement for the desktop OpenAL backend on Android.
pub struct AndroidSound {
    /// CPAL stream – must be kept alive for audio to continue playing.
    stream: cpal::Stream,
    /// Sends filtered, resampled f32 samples to the CPAL callback.
    data_sender: SyncSender<f32>,
    /// Desired playback state shared with the audio callback.
    playing: Arc<AtomicBool>,
    /// Request that the callback discard any queued samples before resuming.
    needs_clear: Arc<AtomicBool>,
    /// Audio filter chain (Nes/Snes).
    filter: AndroidFilter,
    /// Master volume/mute gain in `[0.0, 1.0]`.
    gain: f32,
    /// Resampler from the source rate to the device playback rate.
    resampler: SimpleDownSampler,
    /// Effective source sample rate returned to the core.
    source_sample_rate: u32,
}

// SAFETY: This Android app is packaged with minSdk 28, so CPAL/Oboe uses the
// AAudio path rather than the older OpenSL ES + JNI fallback. `AndroidSound` is
// created on the frontend thread and then moved once into the console thread,
// which is the only thread that calls `play()`, `pause()`, or drops the stream.
// The CPAL/Oboe audio callback runs on the platform audio thread and only
// touches the callback-owned receiver and atomics captured by the closure, not
// the Rust `cpal::Stream` wrapper after the transfer.
unsafe impl Send for AndroidSound {}

enum AndroidFilter {
    Nes(NesFilter),
    Snes(SnesFilter),
}

impl AndroidFilter {
    fn new(profile: AudioFilterProfile, sample_rate: f32) -> Self {
        match profile {
            AudioFilterProfile::Nes => Self::Nes(NesFilter::new(sample_rate)),
            AudioFilterProfile::Snes => Self::Snes(SnesFilter::new(sample_rate)),
        }
    }
}

impl Filter for AndroidFilter {
    fn step(&mut self, data: f32) -> f32 {
        match self {
            Self::Nes(filter) => filter.step(data),
            Self::Snes(filter) => filter.step(data),
        }
    }
}

impl AndroidSound {
    /// Create an `AndroidSound` backend.
    ///
    /// * `requested_playback_sample_rate` – playback rate requested by settings.
    /// * `latency_ms` – target latency requested by settings.
    /// * `output_rate` – the NES CPU clock rate (used as the pre-resampler
    ///   source rate cap).
    /// * `gain` – master volume; `1.0` is full volume, `0.0` is muted.
    ///
    /// Returns `Err` (with a descriptive message) if the audio device or stream
    /// cannot be opened.  Callers must surface this error rather than falling
    /// back silently.
    pub fn with_gain_and_filter(
        requested_playback_sample_rate: i32,
        latency_ms: u16,
        source_sample_rate: i32,
        gain: f32,
        filter_profile: AudioFilterProfile,
    ) -> Result<Self, String> {
        let host = cpal::default_host();

        let device = host
            .default_output_device()
            .ok_or_else(|| "no default audio output device available".to_string())?;

        let supported_config = device
            .default_output_config()
            .map_err(|e| format!("failed to query default audio output configuration: {e}"))?;

        let default_playback_sample_rate = supported_config.sample_rate();
        let playback_sample_rate = u32::try_from(requested_playback_sample_rate).map_err(|_| {
            format!(
                "requested_playback_sample_rate must be non-negative, got {requested_playback_sample_rate}"
            )
        })?;
        let channels = supported_config.channels();
        let requested_buffer_frames = latency_buffer_frames(playback_sample_rate, latency_ms);
        let playing = Arc::new(AtomicBool::new(false));
        let needs_clear = Arc::new(AtomicBool::new(true));

        let requested_source_rate_u32 = u32::try_from(source_sample_rate).map_err(|_| {
            format!("source_sample_rate must be non-negative, got {source_sample_rate}")
        })?;

        // Cap the source rate to at most OVERSAMPLE_FACTOR × the playback rate
        // to bound the amount of work done by the resampler.
        let effective_source_sample_rate = requested_source_rate_u32
            .min(playback_sample_rate.saturating_mul(OVERSAMPLE_FACTOR))
            .max(playback_sample_rate);

        let filter = AndroidFilter::new(filter_profile, playback_sample_rate as f32);
        let mut stream_config = supported_config.config();
        stream_config.sample_rate = playback_sample_rate;
        let configured_buffer_size =
            clamp_buffer_size(requested_buffer_frames, *supported_config.buffer_size());
        let configured_buffer_frames = match configured_buffer_size {
            cpal::BufferSize::Fixed(frames) => frames,
            cpal::BufferSize::Default => requested_buffer_frames,
        };
        let queue_capacity =
            usize::try_from(configured_buffer_frames.max(playback_sample_rate / 10))
                .expect("queue capacity should fit into usize");
        let (data_sender, data_receiver) = sync_channel::<f32>(queue_capacity);
        stream_config.buffer_size = configured_buffer_size;
        let callback_playing = playing.clone();
        let callback_needs_clear = needs_clear.clone();

        log::info!(
            "android audio: device='{}' requested_rate={} default_rate={} channels={} target_latency_ms={} requested_buffer_frames={} configured_buffer_frames={} queue_capacity={}",
            device
                .description()
                .map(|description| description.to_string())
                .unwrap_or_else(|_| "<unknown>".to_string()),
            playback_sample_rate,
            default_playback_sample_rate,
            channels,
            latency_ms,
            requested_buffer_frames,
            configured_buffer_frames,
            queue_capacity,
        );

        let stream = device
            .build_output_stream(
                &stream_config,
                move |output: &mut [f32], _info: &cpal::OutputCallbackInfo| {
                    if callback_needs_clear.swap(false, Ordering::AcqRel) {
                        while data_receiver.try_recv().is_ok() {}
                    }
                    let playing = callback_playing.load(Ordering::Acquire);
                    // Interleave the mono audio across all device channels.
                    for frame in output.chunks_mut(channels as usize) {
                        let sample = if playing {
                            data_receiver.try_recv().unwrap_or(0.0)
                        } else {
                            0.0
                        };
                        for ch in frame.iter_mut() {
                            *ch = sample;
                        }
                    }
                },
                |err| log::error!("android audio stream error: {err}"),
                None,
            )
            .map_err(|e| format!("failed to build android audio stream: {e}"))?;

        Ok(Self {
            stream,
            data_sender,
            playing,
            needs_clear,
            filter,
            gain,
            resampler: SimpleDownSampler::new(
                f64::from(effective_source_sample_rate),
                f64::from(playback_sample_rate),
            ),
            source_sample_rate: effective_source_sample_rate,
        })
    }

    pub fn with_gain(
        requested_playback_sample_rate: i32,
        latency_ms: u16,
        output_rate: i32,
        gain: f32,
    ) -> Result<Self, String> {
        // Backwards-compatible wrapper: default to NES profile
        Self::with_gain_and_filter(
            requested_playback_sample_rate,
            latency_ms,
            output_rate,
            gain,
            AudioFilterProfile::Nes,
        )
    }
}

impl Sound for AndroidSound {
    /// Resume the audio stream.  Maps to the app coming to the foreground.
    fn start(&mut self) {
        self.needs_clear.store(true, Ordering::Release);
        self.playing.store(true, Ordering::Release);
        if let Err(e) = self.stream.play() {
            log::error!("failed to start android audio stream: {e}");
        }
    }

    /// Pause the audio stream.  Maps to the app going to the background.
    fn pause(&mut self) {
        self.playing.store(false, Ordering::Release);
        self.needs_clear.store(true, Ordering::Release);
        if let Err(e) = self.stream.pause() {
            log::warn!("failed to pause android audio stream, keeping callback muted: {e}");
        }
    }
}

impl MixerInput for AndroidSound {
    /// Accept a raw NES mixer sample in `[0.0, 1.0]`, apply resampling and
    /// the NES filter chain, then queue the result for the CPAL callback.
    fn push(&mut self, data: f32) {
        if let Some(resampled) = self.resampler.step(data) {
            // Map [0.0, 1.0] → [-1.0, 1.0], apply gain, then run NES filters.
            let sample = self.filter.step((resampled * 2.0 - 1.0) * self.gain);
            match self.data_sender.try_send(sample) {
                Ok(()) | Err(TrySendError::Full(_)) => {}
                Err(TrySendError::Disconnected(_)) => {
                    log::warn!("android audio: channel send failed (receiver dropped)");
                }
            }
        }
    }

    fn sample_rate(&self) -> u32 {
        self.source_sample_rate
    }
}

fn latency_buffer_frames(playback_sample_rate: u32, latency_ms: u16) -> u32 {
    u32::try_from((u64::from(playback_sample_rate) * u64::from(latency_ms)).div_ceil(1_000))
        .unwrap_or(u32::MAX)
        .max(1)
}

fn clamp_buffer_size(
    requested_frames: u32,
    supported: cpal::SupportedBufferSize,
) -> cpal::BufferSize {
    match supported {
        cpal::SupportedBufferSize::Range { min, max } => {
            cpal::BufferSize::Fixed(requested_frames.clamp(min, max))
        }
        cpal::SupportedBufferSize::Unknown => cpal::BufferSize::Fixed(requested_frames),
    }
}
