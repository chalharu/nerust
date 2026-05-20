use crate::{EncodedNtscTextures, EncodedPackedNtscTexture};
use nerust_screen_traits::{LogicalSize, PhysicalSize};

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

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum VideoPresentationPipelineKind {
    DirectRgba,
    Palette,
    Ntsc,
}

#[derive(Debug, Clone)]
pub enum VideoFilterPipeline {
    DirectRgba,
    Palette {
        palette_rgba8: Box<[u8]>,
    },
    Ntsc {
        palette_rgba8: Box<[u8]>,
        packed_ntsc_rgba8: EncodedPackedNtscTexture,
        split_ntsc_textures: EncodedNtscTextures,
    },
}

impl VideoFilterPipeline {
    pub fn kind(&self) -> VideoPresentationPipelineKind {
        match self {
            Self::DirectRgba => VideoPresentationPipelineKind::DirectRgba,
            Self::Palette { .. } => VideoPresentationPipelineKind::Palette,
            Self::Ntsc { .. } => VideoPresentationPipelineKind::Ntsc,
        }
    }

    pub fn palette_rgba8(&self) -> Option<&[u8]> {
        match self {
            Self::Palette { palette_rgba8 } | Self::Ntsc { palette_rgba8, .. } => {
                Some(palette_rgba8.as_ref())
            }
            Self::DirectRgba => None,
        }
    }

    pub fn packed_ntsc_rgba8(&self) -> Option<&[u8]> {
        match self {
            Self::Ntsc {
                packed_ntsc_rgba8, ..
            } => Some(packed_ntsc_rgba8.rgba8.as_ref()),
            Self::DirectRgba | Self::Palette { .. } => None,
        }
    }

    pub fn split_ntsc_textures(&self) -> Option<&EncodedNtscTextures> {
        match self {
            Self::Ntsc {
                split_ntsc_textures,
                ..
            } => Some(split_ntsc_textures),
            Self::DirectRgba | Self::Palette { .. } => None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct VideoPresentation {
    frame_spec: VideoFrameSpec,
    pipeline: VideoFilterPipeline,
}

impl VideoPresentation {
    pub(crate) fn new(frame_spec: VideoFrameSpec, pipeline: VideoFilterPipeline) -> Self {
        Self {
            frame_spec,
            pipeline,
        }
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

    pub fn pipeline_kind(&self) -> VideoPresentationPipelineKind {
        self.pipeline.kind()
    }

    pub fn is_palette_pipeline(&self) -> bool {
        matches!(self.pipeline.kind(), VideoPresentationPipelineKind::Palette)
    }

    pub fn uses_ntsc_pipeline(&self) -> bool {
        matches!(self.pipeline.kind(), VideoPresentationPipelineKind::Ntsc)
    }

    pub fn palette_rgba8(&self) -> Option<&[u8]> {
        self.pipeline.palette_rgba8()
    }

    pub fn packed_ntsc_rgba8(&self) -> Option<&[u8]> {
        self.pipeline.packed_ntsc_rgba8()
    }

    pub fn split_ntsc_textures(&self) -> Option<&EncodedNtscTextures> {
        self.pipeline.split_ntsc_textures()
    }
}

#[cfg(test)]
mod tests {
    use super::{VideoFrameFormat, VideoFrameSpec};
    use nerust_screen_traits::{LogicalSize, PhysicalSize};

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
        let source_logical_size = spec.source_logical_size();
        assert_eq!(source_logical_size.width, 256);
        assert_eq!(source_logical_size.height, 240);

        let logical_size = spec.logical_size();
        assert_eq!(logical_size.width, 602);
        assert_eq!(logical_size.height, 240);

        let physical_size = spec.physical_size();
        assert_eq!(physical_size.width, 602.0);
        assert_eq!(physical_size.height, 480.0);
    }
}
