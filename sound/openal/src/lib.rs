// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

mod resampler;

use self::resampler::{Resampler, SimpleDownSampler};
#[cfg(feature = "cpal")]
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use nerust_sound_traits::{MixerInput, Sound};
use nerust_soundfilter::{Filter, NesFilter};
use std::f64;
#[cfg(feature = "cpal")]
use std::sync::mpsc::{Receiver, Sender, channel};

#[cfg(feature = "cpal")]
#[derive(Debug)]
struct FadeBuffer {
    data_receiver: Receiver<f32>,
    fade_width: usize,
    fadein_window_lut: Vec<f32>,
    fadeout_window_lut: Vec<f32>,
    fade_buffer: Vec<f32>,
    input_pos: usize,
    output_pos: usize,
    fade_pos: usize,
}

#[cfg(feature = "cpal")]
impl FadeBuffer {
    pub(crate) fn new(data_receiver: Receiver<f32>, fade_width: usize) -> Self {
        // 必ず lut[0] = 0 とする
        let hannning_fadein_window_lut = (0..fade_width)
            .map(|x| 0.5 - ((x as f64 * f64::consts::PI / fade_width as f64).cos() * 0.5) as f32)
            .collect::<Vec<_>>();
        // 必ず lut[0] = 1 とする
        let hannning_fadeout_window_lut = (0..fade_width)
            .map(|x| {
                0.5 - (((x as f64 + fade_width as f64) * f64::consts::PI / fade_width as f64).cos()
                    * 0.5) as f32
            })
            .collect::<Vec<_>>();
        let fade_buffer = vec![0.0; (fade_width * 2 + 1).next_power_of_two()];

        Self {
            data_receiver,
            fade_width,
            fadein_window_lut: hannning_fadein_window_lut,
            fadeout_window_lut: hannning_fadeout_window_lut,
            fade_buffer,
            input_pos: 0,
            output_pos: 0,
            fade_pos: 0,
        }
    }
}

#[cfg(feature = "cpal")]
impl Iterator for FadeBuffer {
    type Item = f32;
    fn next(&mut self) -> Option<f32> {
        let lenmask = self.fade_buffer.len() - 1;
        while (self.output_pos + self.fade_width) & lenmask != self.input_pos {
            if let Ok(data) = self.data_receiver.try_recv() {
                self.fade_buffer[self.input_pos] = data;
                self.input_pos = (self.input_pos + 1) & lenmask;
            } else {
                break;
            }
        }
        Some(
            if (self.output_pos + self.fade_width) & lenmask != self.input_pos || self.fade_pos > 0
            {
                // 入力データが不足している場合
                self.fade_pos += 1;
                if self.fade_pos == self.fade_width {
                    self.fade_pos = 0;
                    self.fade_buffer[self.output_pos]
                } else {
                    let fade_pos = (self.fade_buffer.len() + self.output_pos + self.fade_pos
                        - self.fade_width)
                        & lenmask;
                    let out_pos = (self.output_pos + self.fade_pos) & lenmask;
                    self.fade_buffer[fade_pos] * self.fadein_window_lut[self.fade_pos]
                        + self.fade_buffer[out_pos] * self.fadeout_window_lut[self.fade_pos]
                }
            } else {
                self.output_pos = (self.output_pos + 1) & lenmask;
                self.fade_buffer[self.output_pos]
            },
        )
    }
}

#[cfg(feature = "cpal")]
struct CpalOutputState {
    playing_receiver: Receiver<bool>,
    playing: bool,
    fade_buffer: FadeBuffer,
}

#[cfg(feature = "cpal")]
impl CpalOutputState {
    pub(crate) fn new(
        playing_receiver: Receiver<bool>,
        data_receiver: Receiver<f32>,
        fade_width: usize,
    ) -> Self {
        Self {
            playing: false,
            playing_receiver,
            fade_buffer: FadeBuffer::new(data_receiver, fade_width),
        }
    }

    fn write<T>(&mut self, data: &mut [T], channels: usize)
    where
        T: cpal::Sample + cpal::SizedSample + cpal::FromSample<f32>,
    {
        while let Ok(new_playing) = self.playing_receiver.try_recv() {
            self.playing = new_playing;
        }

        for frame in data.chunks_mut(channels) {
            let sample = if self.playing {
                self.fade_buffer.next().unwrap_or(0.0).clamp(-1.0, 1.0)
            } else {
                0.0
            };
            for output in frame {
                *output = T::from_sample_(sample);
            }
        }
    }
}

#[cfg(feature = "cpal")]
fn write_output<T>(state: &mut CpalOutputState, data: &mut [T], channels: usize)
where
    T: cpal::Sample + cpal::SizedSample + cpal::FromSample<f32>,
{
    state.write(data, channels);
}

#[cfg(feature = "cpal")]
fn build_output_stream<T>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    state: CpalOutputState,
) -> Result<cpal::Stream, cpal::BuildStreamError>
where
    T: cpal::Sample + cpal::SizedSample + cpal::FromSample<f32>,
{
    let channels = usize::from(config.channels);
    let mut state = state;
    device.build_output_stream(
        config,
        move |data: &mut [T], _: &cpal::OutputCallbackInfo| {
            write_output(&mut state, data, channels);
        },
        |err| log::warn!("cpal output stream error: {err}"),
        None,
    )
}

#[cfg(feature = "cpal")]
fn build_stream_for_format(
    device: &cpal::Device,
    sample_format: cpal::SampleFormat,
    config: &cpal::StreamConfig,
    state: CpalOutputState,
) -> Result<cpal::Stream, cpal::BuildStreamError> {
    match sample_format {
        cpal::SampleFormat::F32 => build_output_stream::<f32>(device, config, state),
        cpal::SampleFormat::I16 => build_output_stream::<i16>(device, config, state),
        cpal::SampleFormat::U16 => build_output_stream::<u16>(device, config, state),
        sample_format => {
            log::warn!("Unsupported cpal sample format '{sample_format}'");
            Err(cpal::BuildStreamError::StreamConfigNotSupported)
        }
    }
}

#[cfg(feature = "cpal")]
fn stream_config(
    device: &cpal::Device,
    sample_rate: i32,
    buffer_width: usize,
) -> Option<(cpal::SampleFormat, cpal::StreamConfig)> {
    let requested_sample_rate = u32::try_from(sample_rate).ok()?;
    let requested_buffer_size = u32::try_from(buffer_width).ok()?;

    let mut supported_config = device.default_output_config().ok()?;
    if let Ok(supported_configs) = device.supported_output_configs() {
        for config_range in supported_configs {
            let min_sample_rate = config_range.min_sample_rate().0;
            let max_sample_rate = config_range.max_sample_rate().0;
            if min_sample_rate <= requested_sample_rate && requested_sample_rate <= max_sample_rate
            {
                supported_config =
                    config_range.with_sample_rate(cpal::SampleRate(requested_sample_rate));
                break;
            }
        }
    }

    let sample_format = supported_config.sample_format();
    let mut config = supported_config.config();
    config.sample_rate = cpal::SampleRate(requested_sample_rate);
    config.buffer_size = cpal::BufferSize::Fixed(requested_buffer_size);

    Some((sample_format, config))
}

#[cfg(feature = "cpal")]
fn create_stream(
    sample_rate: i32,
    buffer_width: usize,
    playing_receiver: Receiver<bool>,
    data_receiver: Receiver<f32>,
) -> (Option<cpal::Stream>, i32) {
    let Some(device) = cpal::default_host().default_output_device() else {
        log::error!("No cpal output device present!");
        return (None, sample_rate);
    };

    let Some((sample_format, config)) = stream_config(&device, sample_rate, buffer_width) else {
        log::error!("No supported cpal output stream configuration found!");
        return (None, sample_rate);
    };
    let actual_sample_rate = i32::try_from(config.sample_rate.0).unwrap_or(sample_rate);
    let state = CpalOutputState::new(playing_receiver, data_receiver, buffer_width);
    match build_stream_for_format(&device, sample_format, &config, state) {
        Ok(stream) => {
            if let Err(err) = stream.play() {
                log::warn!("cpal output stream play failed: {err}");
            }
            (Some(stream), actual_sample_rate)
        }
        Err(err) => {
            log::error!("cpal output stream creation failed: {err}");
            (None, sample_rate)
        }
    }
}

#[derive(Debug)]
pub struct OpenAl {
    #[cfg(feature = "cpal")]
    playing_sender: Sender<bool>,
    #[cfg(feature = "cpal")]
    data_sender: Sender<f32>,
    filter: NesFilter,
    #[cfg(feature = "cpal")]
    stream: Option<cpal::Stream>,
    resampler: SimpleDownSampler,
}

impl OpenAl {
    pub fn new(
        sample_rate: i32,
        output_rate: i32,
        buffer_width: usize,
        buffer_count: usize,
    ) -> Self {
        #[cfg(feature = "cpal")]
        {
            let (playing_sender, playing_recv) = channel();
            let (data_sender, data_recv) = channel();
            let _ = buffer_count;
            let (stream, actual_sample_rate) =
                create_stream(sample_rate, buffer_width, playing_recv, data_recv);
            let filter = NesFilter::new(actual_sample_rate as f32);

            Self {
                filter,
                playing_sender,
                data_sender,
                stream,
                resampler: SimpleDownSampler::new(
                    f64::from(output_rate),
                    f64::from(actual_sample_rate),
                ),
            }
        }

        #[cfg(not(feature = "cpal"))]
        {
            let _ = (buffer_width, buffer_count);
            Self {
                filter: NesFilter::new(sample_rate as f32),
                resampler: SimpleDownSampler::new(f64::from(output_rate), f64::from(sample_rate)),
            }
        }
    }
}

impl Sound for OpenAl {
    fn pause(&mut self) {
        #[cfg(feature = "cpal")]
        if self.playing_sender.send(false).is_err() {
            log::warn!("OpenAL channel (playing) send failed");
        }
    }

    fn start(&mut self) {
        #[cfg(feature = "cpal")]
        if self.playing_sender.send(true).is_err() {
            log::warn!("OpenAL channel (playing) send failed");
        }
    }
}

impl MixerInput for OpenAl {
    // 0.0 ~ 1.0 => -1.0 ~ 1.0
    fn push(&mut self, data: f32) {
        if let Some(resampled_data) = self.resampler.step(data) {
            let sample = self.filter.step(resampled_data * 2.0 - 1.0);
            #[cfg(feature = "cpal")]
            if self.data_sender.send(sample).is_err() {
                log::warn!("OpenAL channel (data) send failed");
            }
            #[cfg(not(feature = "cpal"))]
            let _ = sample;
        }
    }
}

#[cfg(feature = "cpal")]
impl Drop for OpenAl {
    fn drop(&mut self) {
        let _ = self.stream.take();
    }
}
