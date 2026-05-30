// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use nerust_screen_logical::LogicalSize;
use nerust_screen_physical::PhysicalSize;

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum VideoFrameFormat {
    Rgba,
    Palette,
}

#[derive(Debug, Clone, Copy)]
pub struct VideoFrameSpec {
    frame_format: VideoFrameFormat,
    source_logical_size: LogicalSize,
    logical_size: LogicalSize,
    physical_size: PhysicalSize,
}

impl VideoFrameSpec {
    pub fn new(
        frame_format: VideoFrameFormat,
        source_logical_size: LogicalSize,
        logical_size: LogicalSize,
        physical_size: PhysicalSize,
    ) -> Self {
        Self {
            frame_format,
            source_logical_size,
            logical_size,
            physical_size,
        }
    }

    pub fn frame_format(&self) -> VideoFrameFormat {
        self.frame_format
    }

    pub fn source_logical_size(&self) -> LogicalSize {
        self.source_logical_size
    }

    pub fn logical_size(&self) -> LogicalSize {
        self.logical_size
    }

    pub fn physical_size(&self) -> PhysicalSize {
        self.physical_size
    }
}

#[derive(Debug, Clone)]
pub struct VideoPresentation {
    frame_spec: VideoFrameSpec,
}

impl VideoPresentation {
    pub fn new(frame_spec: VideoFrameSpec) -> Self {
        Self { frame_spec }
    }

    pub fn source_logical_size(&self) -> LogicalSize {
        self.frame_spec.source_logical_size()
    }

    pub fn logical_size(&self) -> LogicalSize {
        self.frame_spec.logical_size()
    }

    pub fn physical_size(&self) -> PhysicalSize {
        self.frame_spec.physical_size()
    }

    pub fn frame_format(&self) -> VideoFrameFormat {
        self.frame_spec.frame_format()
    }

    pub fn is_palette_frame(&self) -> bool {
        matches!(self.frame_spec.frame_format(), VideoFrameFormat::Palette)
    }
}

pub trait Screen {
    fn push(&mut self, palette: u8);

    #[inline]
    fn push_many(&mut self, palette: u8, count: u16) {
        for _ in 0..count {
            self.push(palette);
        }
    }

    fn render(&mut self);
}
