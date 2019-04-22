// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use super::allocate;
use nerust_screen_filter::FilterFunc;
use nerust_screen_traits::{LogicalSize, RGB};

pub struct ScreenBufferUnit {
    buffer: Box<[u8]>,
    pos: usize,
}

impl ScreenBufferUnit {
    pub fn new(size: LogicalSize) -> Self {
        Self {
            buffer: init_screen_buffer(size),
            pos: 0,
        }
    }

    pub fn reset(&mut self) {
        self.pos = 0;
    }

    pub fn as_ptr(&self) -> *const u8 {
        self.buffer.as_ptr() as *const u8
    }

    pub fn as_mut_ptr(&mut self) -> *mut u8 {
        self.buffer.as_mut_ptr() as *mut u8
    }
}

impl FilterFunc for ScreenBufferUnit {
    fn filter_func(&mut self, color: RGB) {
        let pos = self.pos << 2;
        self.buffer[pos] = color.red;
        self.buffer[pos + 1] = color.green;
        self.buffer[pos + 2] = color.blue;
        self.pos += 1;
    }
}

pub fn init_screen_buffer(size: LogicalSize) -> Box<[u8]> {
    allocate(size.width * size.height * 4)
}

unsafe impl Send for ScreenBufferUnit {}
unsafe impl Sync for ScreenBufferUnit {}
