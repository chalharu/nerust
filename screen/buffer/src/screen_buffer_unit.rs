// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use super::allocate;
use nerust_screen_filter::FilterFunc;
use nerust_screen_traits::{LogicalSize, RGB};

pub struct ScreenBufferUnit {
    buffer: Box<[u32]>,
    pos: usize,
}

impl ScreenBufferUnit {
    pub fn new(size: LogicalSize) -> Self {
        Self {
            buffer: init_screen_buffer(size),
            pos: 0,
        }
    }

    #[inline]
    pub fn reset(&mut self) {
        self.pos = 0;
    }

    #[inline]
    pub fn as_ptr(&self) -> *const u8 {
        self.buffer.as_ptr() as *const u32 as *const u8
    }

    #[inline]
    pub fn as_mut_ptr(&mut self) -> *mut u8 {
        self.buffer.as_mut_ptr() as *mut u8
    }

    #[inline]
    pub fn clear(&mut self) {
        for b in self.buffer.iter_mut() {
            *b = 0;
        }
        self.pos = 0;
    }
}

impl FilterFunc for ScreenBufferUnit {
    #[inline]
    fn filter_func(&mut self, color: RGB) {
        unsafe {
            *(self.buffer.get_unchecked_mut(self.pos)) =
                u32::from(color.red) | u32::from(color.green) << 8 | u32::from(color.blue) << 16;
        }
        self.pos += 1;
    }
}

#[inline]
pub fn init_screen_buffer(size: LogicalSize) -> Box<[u32]> {
    allocate(size.width * size.height)
}

unsafe impl Send for ScreenBufferUnit {}
unsafe impl Sync for ScreenBufferUnit {}
