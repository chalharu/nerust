// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use super::allocate;
use nerust_screen_filter::FilterFunc;
use nerust_screen_traits::{LogicalSize, RGB};
use std::{mem, slice};

const OPAQUE_BLACK: u32 = 0xFF00_0000;

#[derive(Debug)]
pub(crate) struct ScreenBufferUnit {
    buffer: Box<[u32]>,
    pos: usize,
}

impl ScreenBufferUnit {
    pub(crate) fn new(size: LogicalSize) -> Self {
        let mut result = Self {
            buffer: init_screen_buffer(size),
            pos: 0,
        };
        result.clear();
        result
    }

    #[inline]
    pub(crate) fn reset(&mut self) {
        self.pos = 0;
    }

    #[inline]
    pub(crate) fn byte_len(&self) -> usize {
        self.buffer.len() * mem::size_of::<u32>()
    }

    #[inline]
    pub(crate) fn copy_to_slice(&self, dest: &mut [u8]) {
        let src =
            unsafe { slice::from_raw_parts(self.buffer.as_ptr().cast::<u8>(), self.byte_len()) };
        assert_eq!(dest.len(), src.len(), "display buffer size mismatch");
        dest.copy_from_slice(src);
    }

    #[inline]
    pub(crate) fn clear(&mut self) {
        for b in self.buffer.iter_mut() {
            *b = OPAQUE_BLACK;
        }
        self.pos = 0;
    }
}

impl FilterFunc for ScreenBufferUnit {
    #[inline]
    fn filter_func(&mut self, color: RGB) {
        unsafe {
            *(self.buffer.get_unchecked_mut(self.pos)) = OPAQUE_BLACK
                | u32::from(color.red)
                | u32::from(color.green) << 8
                | u32::from(color.blue) << 16;
        }
        self.pos += 1;
    }
}

#[inline]
pub(crate) fn init_screen_buffer(size: LogicalSize) -> Box<[u32]> {
    allocate(size.width * size.height)
}

unsafe impl Send for ScreenBufferUnit {}
