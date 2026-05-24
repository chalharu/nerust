use crate::filter::{EncodedNtscTextures, EncodedPackedNtscTexture};

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum VideoPresentationPipelineKind {
    Palette,
    Ntsc,
}

#[derive(Debug, Clone)]
pub enum VideoFilterPipeline {
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
            Self::Palette { .. } => VideoPresentationPipelineKind::Palette,
            Self::Ntsc { .. } => VideoPresentationPipelineKind::Ntsc,
        }
    }

    pub fn palette_rgba8(&self) -> &[u8] {
        match self {
            Self::Palette { palette_rgba8 } | Self::Ntsc { palette_rgba8, .. } => {
                palette_rgba8.as_ref()
            }
        }
    }

    pub fn packed_ntsc_rgba8(&self) -> Option<&[u8]> {
        match self {
            Self::Ntsc {
                packed_ntsc_rgba8, ..
            } => Some(packed_ntsc_rgba8.rgba8.as_ref()),
            Self::Palette { .. } => None,
        }
    }

    pub fn split_ntsc_textures(&self) -> Option<&EncodedNtscTextures> {
        match self {
            Self::Ntsc {
                split_ntsc_textures,
                ..
            } => Some(split_ntsc_textures),
            Self::Palette { .. } => None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct NesVideoAssets {
    pipeline: VideoFilterPipeline,
}

/// Console-family video asset contract.
///
/// Each variant carries the GPU upload data for one console family.
/// The current implementation only covers the NES; a future SNES variant
/// would be added here without touching the shared rendering layers.
#[derive(Debug, Clone)]
pub enum ConsoleVideoAssets {
    /// NES palette / NTSC shader textures.
    Nes(NesVideoAssets),
    // Future: Snes(SnesVideoAssets),
}

impl ConsoleVideoAssets {
    /// Return the inner [`NesVideoAssets`] if this is the NES variant.
    pub fn as_nes(&self) -> Option<&NesVideoAssets> {
        match self {
            Self::Nes(assets) => Some(assets),
        }
    }

    /// Convenience delegate: palette RGBA8 data regardless of console family.
    pub fn palette_rgba8(&self) -> &[u8] {
        match self {
            Self::Nes(assets) => assets.palette_rgba8(),
        }
    }
}

impl NesVideoAssets {
    pub(crate) fn new(pipeline: VideoFilterPipeline) -> Self {
        Self { pipeline }
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

    pub fn palette_rgba8(&self) -> &[u8] {
        self.pipeline.palette_rgba8()
    }

    pub fn packed_ntsc_rgba8(&self) -> Option<&[u8]> {
        self.pipeline.packed_ntsc_rgba8()
    }

    pub fn split_ntsc_textures(&self) -> Option<&EncodedNtscTextures> {
        self.pipeline.split_ntsc_textures()
    }
}
