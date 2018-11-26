// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

mod filter;

use self::filter::*;
use alto::*;
use crate::nes::MixerInput;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::thread;
use std::thread::JoinHandle;
use std::time::Duration;
use std::{f64, i16, mem};

pub trait Sound {
    fn start(&mut self);
    fn pause(&mut self);
}

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
    pub fn new(data_receiver: Receiver<f32>, fade_width: usize) -> Self {
        // 必ず lut[0] = 0 とする
        let hannning_fadein_window_lut = (0..fade_width)
            .map(|x| 0.5 - ((x as f64 * f64::consts::PI / fade_width as f64).cos() * 0.5) as f32)
            .collect::<Vec<_>>();
        // 必ず lut[0] = 1 とする
        let hannning_fadeout_window_lut = (0..fade_width)
            .map(|x| {
                0.5 - (((x as f64 + fade_width as f64) * f64::consts::PI / fade_width as f64).cos()
                    * 0.5) as f32
            }).collect::<Vec<_>>();
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
    pub fn new(
        sample_rate: i32,
        buffer_width: usize,
        buffer_count: usize,
        playing_receiver: Receiver<bool>,
        data_receiver: Receiver<f32>,
        fade_width: usize,
    ) -> Self {
        let src = if let Ok(src) = Alto::load_default()
            .and_then(|alto| alto.open(None))
            .and_then(|dev| dev.new_context(None))
            .and_then(|ctx| ctx.new_streaming_source().map(|src| (src, ctx)))
            .and_then(|(mut src, mut ctx)| {
                for _ in 0..buffer_count {
                    Self::add_buffer(&mut ctx, &mut src, sample_rate, buffer_width);
                }
                Ok(src)
            }) {
            Some(src)
        } else {
            error!("No OpenAL implementation present!");
            None
        };
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
    ) {
        let data = &vec![Mono { center: 0_i16 }; buffer_width];
        let buf = ctx.new_buffer(data, sample_rate).unwrap();
        src.queue_buffer(buf).unwrap();
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
                center: (d * i16::max_value() as f32) as i16,
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
            } else {
                match src.state() {
                    SourceState::Playing => src.pause(),
                    _ => (),
                }
            }
        }
    }
}

pub struct OpenAl {
    rate: f64,
    cycle: f64,
    next_cycle: f64,
    stop_sender: Sender<()>,
    playing_sender: Sender<bool>,
    data_sender: Sender<f32>,
    filter: NesFilter,
    thread: Option<JoinHandle<()>>,
}

impl OpenAl {
    pub fn new(sample_rate: i32, buffer_width: usize, buffer_count: usize) -> Self {
        let rate = super::CLOCK_RATE as f64 / sample_rate as f64;
        let filter = NesFilter::new(sample_rate as f32);
        let (playing_sender, playing_recv) = channel();
        let (data_sender, data_recv) = channel();
        let (stop_sender, stop_recv) = channel();
        let thread = thread::spawn(move || {
            let mut state = OpenAlState::new(
                sample_rate,
                buffer_width,
                buffer_count,
                playing_recv,
                data_recv,
                buffer_width,
            );
            while let Err(_) = stop_recv.try_recv() {
                state.step();
                thread::sleep(Duration::from_millis(1));
            }
        });

        Self {
            filter,
            rate,
            cycle: 0.0,
            next_cycle: 0.0,
            playing_sender,
            data_sender,
            stop_sender,
            thread: Some(thread),
        }
    }
}

impl Sound for OpenAl {
    fn pause(&mut self) {
        if let Err(_) = self.playing_sender.send(false) {
            warn!("OpenAL channel (playing) send failed");
        }
    }

    fn start(&mut self) {
        if let Err(_) = self.playing_sender.send(true) {
            warn!("OpenAL channel (playing) send failed");
        }
    }
}

impl MixerInput for OpenAl {
    // 0.0 ~ 1.0 => -1.0 ~ 1.0
    fn push(&mut self, data: f32) {
        self.cycle += 1.0;
        if self.cycle > self.next_cycle {
            self.next_cycle += self.rate;
            if let Err(_) = self.data_sender.send(self.filter.step(data * 2.0 - 1.0)) {
                warn!("OpenAL channel (data) send failed");
            }
        }
    }
}

impl Drop for OpenAl {
    fn drop(&mut self) {
        if let Err(_) = self.stop_sender.send(()) {
            warn!("OpenAL channel (stop) send failed");
        }
        mem::replace(&mut self.thread, None).map(|x| x.join());
    }
}
