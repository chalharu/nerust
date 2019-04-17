// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::collections::VecDeque;
use std::ops::Add;
use std::thread;
use std::time::{Duration, Instant};

pub const CLOCK_RATE: usize = 1_789_773;

pub struct Timer {
    instants: VecDeque<Instant>,
    wait_instants: Instant,
    thread_sleep_nanos: Duration,
    frame_wait_nanos: Duration,
}

impl Timer {
    pub fn new() -> Self {
        let instants = VecDeque::with_capacity(Self::CALC_FRAMES);
        let wait_instants = Instant::now();
        Self {
            instants,
            wait_instants,
            thread_sleep_nanos: Duration::from_nanos(Self::FRAME_WAIT_NANOS - 1_000_000),
            frame_wait_nanos: Duration::from_nanos(Self::FRAME_WAIT_NANOS),
        }
    }

    const CALC_FRAMES: usize = 64;
    const FRAME_DOTS: f64 = 89341.5;
    const VSYNC_RATE: f64 = CLOCK_RATE as f64 * 3.0 / Self::FRAME_DOTS;
    const FRAME_WAIT_NANOS: u64 = (1.0 / Self::VSYNC_RATE * 1_000_000_000.0) as u64;

    pub fn wait(&mut self) {
        let new_now = Instant::now();
        let duration = new_now.duration_since(self.wait_instants);
        if let Some(wait) = self.thread_sleep_nanos.checked_sub(duration) {
            thread::sleep(wait);
        }
        let next = self.wait_instants.add(self.frame_wait_nanos);
        let mut wait_instants = Instant::now();
        while wait_instants < next {
            wait_instants = Instant::now();
        }
        self.wait_instants = wait_instants;
    }

    pub fn as_fps(&mut self) -> f32 {
        let new_now = Instant::now();
        let len = self.instants.len();
        if len == 0 {
            self.instants.push_back(new_now);
            return 0.0;
        }
        let duration = new_now.duration_since(if len >= Self::CALC_FRAMES {
            self.instants.pop_front().unwrap()
        } else {
            *self.instants.front().unwrap()
        });
        self.instants.push_back(new_now);
        (1_000_000_000_f64
            / (duration.as_secs() * 1_000_000_000 + u64::from(duration.subsec_nanos())) as f64
            * len as f64) as f32
    }
}
