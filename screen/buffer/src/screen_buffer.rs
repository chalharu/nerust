// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use super::allocate;
use super::screen_buffer_unit::ScreenBufferUnit;
use nerust_screen_filter::{FilterType, NesFilter};
use nerust_screen_traits::{LogicalSize, PhysicalSize, Screen};
use std::hash::{Hash, Hasher};
use std::mem;

pub struct ScreenBuffer {
    filter: Box<dyn NesFilter>,
    dest: ScreenBufferUnit,
    display_buffer: ScreenBufferUnit,
    src_buffer: Box<[u8]>,
    src_buffer_next: Box<[u8]>,
    src_pos: usize,
}

impl ScreenBuffer {
    pub fn new(filter_type: FilterType, src_size: LogicalSize) -> Self {
        let filter = filter_type.generate(src_size);
        let src_buffer_size = src_size.height * src_size.width;
        let src_buffer = allocate(src_buffer_size);
        let src_buffer_next = allocate(src_buffer_size);

        Self {
            dest: ScreenBufferUnit::new(filter.logical_size()),
            display_buffer: ScreenBufferUnit::new(filter.logical_size()),
            filter,
            src_buffer,
            src_buffer_next,
            src_pos: 0,
        }
    }

    pub fn frame_len(&self) -> usize {
        self.display_buffer.byte_len()
    }

    pub fn copy_display_buffer(&self, dest: &mut [u8]) {
        self.display_buffer.copy_to_slice(dest);
    }

    pub fn logical_size(&self) -> LogicalSize {
        self.filter.logical_size()
    }

    pub fn physical_size(&self) -> PhysicalSize {
        self.filter.physical_size()
    }

    pub fn clear(&mut self) {
        self.dest.clear();
        self.display_buffer.clear();
        for b in self.src_buffer.iter_mut() {
            *b = 0;
        }
        for b in self.src_buffer_next.iter_mut() {
            *b = 0;
        }
    }
}

impl Screen for ScreenBuffer {
    fn push(&mut self, value: u8) {
        let dest = &mut self.dest;
        self.filter.as_mut().push(value, dest);
        self.src_buffer_next[self.src_pos] = value;
        self.src_pos += 1;
    }

    fn render(&mut self) {
        assert_eq!(
            self.src_pos,
            self.src_buffer_next.len(),
            "source frame size mismatch before publish"
        );
        assert_eq!(
            self.dest.pixel_len(),
            self.display_buffer.pixel_len(),
            "display buffer sizes diverged"
        );
        assert!(
            self.dest.is_full(),
            "filtered frame size mismatch before publish"
        );
        mem::swap(&mut self.src_buffer, &mut self.src_buffer_next);
        mem::swap(&mut self.dest, &mut self.display_buffer);
        self.src_pos = 0;
        self.dest.reset();
    }
}

impl Hash for ScreenBuffer {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.src_buffer.hash(state);
    }
}

#[cfg(test)]
mod tests {
    use super::ScreenBuffer;
    use nerust_screen_filter::FilterType;
    use nerust_screen_traits::{LogicalSize, Screen};

    #[test]
    fn all_filters_publish_full_frames() {
        let source = LogicalSize {
            width: 256,
            height: 240,
        };
        for filter in [
            FilterType::None,
            FilterType::NtscRGB,
            FilterType::NtscComposite,
            FilterType::NtscSVideo,
        ] {
            let mut screen = ScreenBuffer::new(filter, source);
            for _ in 0..(source.width * source.height) {
                screen.push(0);
            }
            screen.render();
        }
    }
}
