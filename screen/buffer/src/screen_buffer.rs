// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use super::allocate;
use super::screen_buffer_unit::ScreenBufferUnit;
use nerust_screen_filter::{BLACK_PALETTE_INDEX, FilterType, NesFilter, VideoPresentation};
use nerust_screen_traits::{LogicalSize, PhysicalSize, Screen, VideoFrameFormat};
use std::hash::{Hash, Hasher};
use std::mem;

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
enum PublishMode {
    FilteredRgba,
    SourcePalette,
}

pub struct ScreenBuffer {
    filter_type: FilterType,
    video_presentation: VideoPresentation,
    publish_mode: PublishMode,
    filter: Option<Box<dyn NesFilter>>,
    dest: Option<ScreenBufferUnit>,
    display_buffer: Option<ScreenBufferUnit>,
    src_buffer: Box<[u8]>,
    src_buffer_next: Box<[u8]>,
    src_pos: usize,
}

impl ScreenBuffer {
    pub fn new(filter_type: FilterType, src_size: LogicalSize) -> Self {
        Self::with_publish_mode(
            filter_type,
            src_size,
            PublishMode::FilteredRgba,
            filter_type.presentation(src_size, VideoFrameFormat::Rgba),
        )
    }

    pub fn new_gpu(filter_type: FilterType, src_size: LogicalSize) -> Self {
        Self::with_publish_mode(
            filter_type,
            src_size,
            PublishMode::SourcePalette,
            filter_type.presentation(src_size, VideoFrameFormat::Palette),
        )
    }

    fn with_publish_mode(
        filter_type: FilterType,
        src_size: LogicalSize,
        publish_mode: PublishMode,
        video_presentation: VideoPresentation,
    ) -> Self {
        let src_buffer_size = src_size.height * src_size.width;
        let src_buffer = allocate(src_buffer_size);
        let src_buffer_next = allocate(src_buffer_size);
        let (filter, dest, display_buffer) = match publish_mode {
            PublishMode::FilteredRgba => {
                let filter = filter_type.generate(src_size);
                let logical_size = filter.logical_size();
                (
                    Some(filter),
                    Some(ScreenBufferUnit::new(logical_size)),
                    Some(ScreenBufferUnit::new(logical_size)),
                )
            }
            PublishMode::SourcePalette => (None, None, None),
        };

        let mut result = Self {
            filter_type,
            video_presentation,
            publish_mode,
            filter,
            src_buffer,
            src_buffer_next,
            dest,
            display_buffer,
            src_pos: 0,
        };
        result.clear();
        result
    }

    pub fn frame_len(&self) -> usize {
        match self.publish_mode {
            PublishMode::FilteredRgba => self
                .display_buffer
                .as_ref()
                .expect("filtered buffers should exist for CPU screen buffers")
                .byte_len(),
            PublishMode::SourcePalette => self.src_buffer.len(),
        }
    }

    pub fn copy_display_buffer(&self, dest: &mut [u8]) {
        self.copy_frame_buffer(dest);
    }

    pub fn copy_frame_buffer(&self, dest: &mut [u8]) {
        match self.publish_mode {
            PublishMode::FilteredRgba => self
                .display_buffer
                .as_ref()
                .expect("filtered buffers should exist for CPU screen buffers")
                .copy_to_slice(dest),
            PublishMode::SourcePalette => self.copy_source_buffer(dest),
        }
    }

    pub fn source_frame_len(&self) -> usize {
        self.src_buffer.len()
    }

    pub fn copy_source_buffer(&self, dest: &mut [u8]) {
        assert_eq!(
            dest.len(),
            self.src_buffer.len(),
            "source buffer size mismatch"
        );
        dest.copy_from_slice(&self.src_buffer);
    }

    pub fn logical_size(&self) -> LogicalSize {
        self.video_presentation.logical_size()
    }

    pub fn source_logical_size(&self) -> LogicalSize {
        self.video_presentation.source_logical_size()
    }

    pub fn filter_type(&self) -> FilterType {
        self.filter_type
    }

    pub fn physical_size(&self) -> PhysicalSize {
        self.video_presentation.physical_size()
    }

    pub fn publishes_palette_frame(&self) -> bool {
        matches!(self.publish_mode, PublishMode::SourcePalette)
    }

    pub fn video_presentation(&self) -> &VideoPresentation {
        &self.video_presentation
    }

    pub fn clear(&mut self) {
        if let Some(dest) = self.dest.as_mut() {
            dest.clear();
        }
        if let Some(display_buffer) = self.display_buffer.as_mut() {
            display_buffer.clear();
        }
        self.src_buffer.fill(BLACK_PALETTE_INDEX);
        self.src_buffer_next.fill(BLACK_PALETTE_INDEX);
        self.src_pos = 0;
    }
}

impl Screen for ScreenBuffer {
    fn push(&mut self, value: u8) {
        if let (Some(filter), Some(dest)) = (self.filter.as_mut(), self.dest.as_mut()) {
            filter.push(value, dest);
        }
        self.src_buffer_next[self.src_pos] = value;
        self.src_pos += 1;
    }

    fn render(&mut self) {
        assert_eq!(
            self.src_pos,
            self.src_buffer_next.len(),
            "source frame size mismatch before publish"
        );
        mem::swap(&mut self.src_buffer, &mut self.src_buffer_next);
        self.src_pos = 0;
        if let (Some(dest), Some(display_buffer)) =
            (self.dest.as_mut(), self.display_buffer.as_mut())
        {
            assert_eq!(
                dest.pixel_len(),
                display_buffer.pixel_len(),
                "display buffer sizes diverged"
            );
            assert!(
                dest.is_full(),
                "filtered frame size mismatch before publish"
            );
            mem::swap(dest, display_buffer);
            dest.reset();
        }
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
    use nerust_screen_filter::{FilterType, VideoPresentationPipelineKind};
    use nerust_screen_traits::{LogicalSize, Screen, VideoFrameFormat};

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

    #[test]
    fn gpu_screen_buffer_publishes_palette_frames() {
        let source = LogicalSize {
            width: 4,
            height: 2,
        };
        let mut screen = ScreenBuffer::new_gpu(FilterType::NtscComposite, source);
        for value in 0..(source.width * source.height) {
            screen.push(value as u8);
        }
        screen.render();

        let mut frame = vec![0; screen.frame_len()];
        screen.copy_frame_buffer(&mut frame);
        assert_eq!(frame, (0..8).map(|value| value as u8).collect::<Vec<_>>());
        assert_eq!(
            screen.video_presentation().frame_format(),
            VideoFrameFormat::Palette
        );
        assert_eq!(
            screen.video_presentation().pipeline_kind(),
            VideoPresentationPipelineKind::Ntsc
        );
    }
}
