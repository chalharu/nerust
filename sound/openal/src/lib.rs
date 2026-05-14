// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

mod resampler;

use self::resampler::{Resampler, SimpleDownSampler};
use alto::*;
use nerust_sound_traits::{MixerInput, Sound};
use nerust_soundfilter::{Filter, NesFilter};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::thread::JoinHandle;
use std::time::Duration;
use std::{f64, thread};

#[cfg(target_os = "macos")]
const MACOS_OPENAL_CANDIDATES: &[&str] = &[
    "/System/Library/Frameworks/OpenAL.framework/OpenAL",
    "/System/Library/Frameworks/OpenAL.framework/Versions/Current/OpenAL",
    "/System/Library/Frameworks/OpenAL.framework/Versions/A/OpenAL",
];

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

struct OpenAlState {
    // alto: Option<Alto>,
    // dev: Option<OutputDevice>,
    // ctx: Option<Context>,
    src: Option<StreamingSource>,
    playing_receiver: Receiver<bool>,
    sample_rate: i32,
    playing: bool,
    fade_buffer: FadeBuffer,
    buffer: Vec<Mono<i16>>,
}

impl OpenAlState {
    fn load_alto() -> Result<Alto, String> {
        OpenAl::with_sanitized_dyld_env(|| {
            let mut errors = Vec::new();

            match Alto::load_default() {
                Ok(alto) => {
                    log::info!("loaded OpenAL with the default loader");
                    return Ok(alto);
                }
                Err(err) => errors.push(format!("default loader failed: {err:?}")),
            }

            #[cfg(target_os = "macos")]
            for path in MACOS_OPENAL_CANDIDATES {
                match Alto::load(path) {
                    Ok(alto) => {
                        log::info!("loaded OpenAL from {path}");
                        return Ok(alto);
                    }
                    Err(err) => errors.push(format!("{path}: {err:?}")),
                }
            }

            Err(format!("failed to load OpenAL: {}", errors.join(" | ")))
        })
    }

    fn create_streaming_source(
        sample_rate: i32,
        buffer_width: usize,
        buffer_count: usize,
    ) -> Result<StreamingSource, String> {
        let alto = Self::load_alto()?;
        let dev = alto
            .open(None)
            .map_err(|err| format!("failed to open default OpenAL output device: {err:?}"))?;
        let mut ctx = dev
            .new_context(None)
            .map_err(|err| format!("failed to create OpenAL context: {err:?}"))?;
        let mut src = ctx
            .new_streaming_source()
            .map_err(|err| format!("failed to create OpenAL streaming source: {err:?}"))?;
        for _ in 0..buffer_count {
            Self::add_buffer(&mut ctx, &mut src, sample_rate, buffer_width)
                .map_err(|err| format!("failed to queue initial OpenAL buffer: {err:?}"))?;
        }
        Ok(src)
    }

    pub(crate) fn new(
        sample_rate: i32,
        buffer_width: usize,
        playing_receiver: Receiver<bool>,
        data_receiver: Receiver<f32>,
        fade_width: usize,
        src: Option<StreamingSource>,
    ) -> Self {
        Self {
            src,
            sample_rate,
            playing: false,
            playing_receiver,
            fade_buffer: FadeBuffer::new(data_receiver, fade_width),
            buffer: vec![Mono { center: 0 }; buffer_width],
        }
    }

    fn add_buffer(
        ctx: &mut Context,
        src: &mut StreamingSource,
        sample_rate: i32,
        buffer_width: usize,
    ) -> AltoResult<()> {
        let data = vec![Mono { center: 0_i16 }; buffer_width];
        let buf = ctx.new_buffer(&data, sample_rate)?;
        src.queue_buffer(buf)?;
        Ok(())
    }

    fn fill_buffer(
        src: &mut StreamingSource,
        sample_rate: i32,
        fade_buffer: &mut FadeBuffer,
        buffer: &mut Vec<Mono<i16>>,
    ) {
        let mut buf = src.unqueue_buffer().unwrap();
        let len = buffer.len();
        buffer.clear();
        for d in fade_buffer.take(len) {
            buffer.push(Mono {
                center: (d * f32::from(i16::MAX)) as i16,
            });
        }
        buf.set_data(buffer, sample_rate).unwrap();
        src.queue_buffer(buf).unwrap();
    }

    fn step(&mut self) {
        if let Ok(new_playing) = self.playing_receiver.try_recv() {
            self.playing = new_playing;
        }
        if let Some(ref mut src) = self.src.as_mut() {
            if self.playing {
                let buffers_processed = src.buffers_processed();
                for _ in 0..buffers_processed {
                    Self::fill_buffer(
                        src,
                        self.sample_rate,
                        &mut self.fade_buffer,
                        &mut self.buffer,
                    );
                }
                match src.state() {
                    SourceState::Playing => (),
                    _ => src.play(),
                }
            } else if let SourceState::Playing = src.state() {
                src.pause();
            }
        }
    }
}

#[derive(Debug)]
pub struct OpenAl {
    stop_sender: Sender<()>,
    playing_sender: Sender<bool>,
    data_sender: Sender<f32>,
    filter: NesFilter,
    thread: Option<JoinHandle<()>>,
    resampler: SimpleDownSampler,
}

impl OpenAl {
    pub fn new(
        sample_rate: i32,
        output_rate: i32,
        buffer_width: usize,
        buffer_count: usize,
    ) -> Self {
        let filter = NesFilter::new(sample_rate as f32);
        let (playing_sender, playing_recv) = channel();
        let (data_sender, data_recv) = channel();
        let (stop_sender, stop_recv) = channel();
        // On macOS, loading Apple's deprecated OpenAL framework from a background thread can
        // race with AppKit/ImageIO initialization. Initialize the backend on the caller thread
        // first, then hand the fully created streaming source to the audio thread.
        let src =
            match OpenAlState::create_streaming_source(sample_rate, buffer_width, buffer_count) {
                Ok(src) => Some(src),
                Err(err) => {
                    log::error!("{err}");
                    None
                }
            };
        let thread = thread::spawn(move || {
            let mut state = OpenAlState::new(
                sample_rate,
                buffer_width,
                playing_recv,
                data_recv,
                buffer_width,
                src,
            );
            while stop_recv.try_recv().is_err() {
                state.step();
                thread::sleep(Duration::from_millis(1));
            }
        });

        Self {
            filter,
            playing_sender,
            data_sender,
            stop_sender,
            thread: Some(thread),
            resampler: SimpleDownSampler::new(f64::from(output_rate), f64::from(sample_rate)),
        }
    }

    #[cfg(target_os = "macos")]
    fn with_sanitized_dyld_env<T>(f: impl FnOnce() -> Result<T, String>) -> Result<T, String> {
        const DYLD_ENV_VARS: [&str; 2] = ["DYLD_LIBRARY_PATH", "DYLD_FALLBACK_LIBRARY_PATH"];

        let saved = DYLD_ENV_VARS.map(|name| (name, std::env::var_os(name)));
        for (name, value) in &saved {
            if value.is_some() {
                log::warn!(
                    "temporarily clearing {name} while loading OpenAL to avoid ImageIO plugin conflicts"
                );
                // SAFETY: OpenAl::new initializes the audio backend on the caller thread before
                // the dedicated audio thread is spawned, so no other thread concurrently mutates
                // these DYLD variables during this narrow load window.
                unsafe {
                    std::env::remove_var(name);
                }
            }
        }

        let result = f();

        for (name, value) in saved {
            match value {
                Some(value) => {
                    // SAFETY: See the rationale above; restoration happens on the same thread
                    // before the audio worker is started, so there is no concurrent env access.
                    unsafe {
                        std::env::set_var(name, value);
                    }
                }
                None => {
                    // SAFETY: See the rationale above; restoration happens before spawning the
                    // audio thread, so there is no concurrent env access.
                    unsafe {
                        std::env::remove_var(name);
                    }
                }
            }
        }

        result
    }

    #[cfg(not(target_os = "macos"))]
    fn with_sanitized_dyld_env<T>(f: impl FnOnce() -> Result<T, String>) -> Result<T, String> {
        f()
    }
}

impl Sound for OpenAl {
    fn pause(&mut self) {
        if self.playing_sender.send(false).is_err() {
            log::warn!("OpenAL channel (playing) send failed");
        }
    }

    fn start(&mut self) {
        if self.playing_sender.send(true).is_err() {
            log::warn!("OpenAL channel (playing) send failed");
        }
    }
}

impl MixerInput for OpenAl {
    // 0.0 ~ 1.0 => -1.0 ~ 1.0
    fn push(&mut self, data: f32) {
        if let Some(resampled_data) = self.resampler.step(data)
            && self
                .data_sender
                .send(self.filter.step(resampled_data * 2.0 - 1.0))
                .is_err()
        {
            log::warn!("OpenAL channel (data) send failed");
        }
    }
}

impl Drop for OpenAl {
    fn drop(&mut self) {
        if self.stop_sender.send(()).is_err() {
            log::warn!("OpenAL channel (stop) send failed");
        }
        let _ = self.thread.take().map(JoinHandle::join);
    }
}
