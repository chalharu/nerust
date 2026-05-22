// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

mod logical_size;
mod physical_size;
mod rgb;

pub use logical_size::LogicalSize;
pub use physical_size::PhysicalSize;
pub use rgb::RGB;

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
    fn render(&mut self);
}

#[cfg(test)]
mod tests {
    use super::{LogicalSize, PhysicalSize, VideoFrameFormat, VideoFrameSpec, VideoPresentation};

    #[test]
    fn video_frame_spec_accessors_match_constructor_inputs() {
        let spec = VideoFrameSpec::new(
            VideoFrameFormat::Palette,
            LogicalSize {
                width: 256,
                height: 240,
            },
            LogicalSize {
                width: 602,
                height: 240,
            },
            PhysicalSize {
                width: 602.0,
                height: 480.0,
            },
        );

        assert_eq!(spec.frame_format(), VideoFrameFormat::Palette);
        assert_eq!(spec.source_logical_size().width, 256);
        assert_eq!(spec.source_logical_size().height, 240);
        assert_eq!(spec.logical_size().width, 602);
        assert_eq!(spec.logical_size().height, 240);
        assert_eq!(spec.physical_size().width, 602.0);
        assert_eq!(spec.physical_size().height, 480.0);
    }

    #[test]
    fn video_presentation_exposes_only_generic_frame_metadata() {
        let presentation = VideoPresentation::new(VideoFrameSpec::new(
            VideoFrameFormat::Rgba,
            LogicalSize {
                width: 256,
                height: 240,
            },
            LogicalSize {
                width: 256,
                height: 240,
            },
            PhysicalSize {
                width: 292.57,
                height: 240.0,
            },
        ));

        assert_eq!(presentation.frame_format(), VideoFrameFormat::Rgba);
        assert!(!presentation.is_palette_frame());
        assert_eq!(presentation.source_logical_size().width, 256);
        assert_eq!(presentation.logical_size().height, 240);
        assert_eq!(presentation.physical_size().height, 240.0);
    }
}
