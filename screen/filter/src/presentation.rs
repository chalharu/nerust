use crate::{EncodedNtscTextures, EncodedPackedNtscTexture};
use nerust_screen_traits::video_frame::{VideoFrameFormat, VideoFrameSpec};
use nerust_screen_traits::{LogicalSize, PhysicalSize};

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
