use crate::filters;
use crate::{FilterType, NTSC_TEXTURE_HEIGHT, PALETTE_TEXTURE_WIDTH};
use nerust_screen_logical::LogicalSize;
use nerust_screen_physical::PhysicalSize;
use nerust_screen_rgb::RGB;
use nerust_screen_video::{VideoFrameFormat, VideoFrameSpec, VideoPresentation};
use nes_ntsc::{ShaderKernelEntry, setup::Setup};

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum VideoPresentationPipelineKind {
    Palette,
    Ntsc,
}

#[derive(Debug, Clone, Copy)]
pub struct FilterLayout {
    pub source_logical_size: LogicalSize,
    pub logical_size: LogicalSize,
    pub physical_size: PhysicalSize,
}

#[derive(Debug, Clone)]
pub struct EncodedNtscTextures {
    pub primary_rgba8: Box<[u8]>,
    pub secondary_rgba8: Box<[u8]>,
}

#[derive(Debug, Clone)]
pub struct EncodedPackedNtscTexture {
    pub rgba8: Box<[u8]>,
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

    /// Convenience delegate: packed NTSC texture data when the active pipeline needs it.
    pub fn packed_ntsc_rgba8(&self) -> Option<&[u8]> {
        match self {
            Self::Nes(assets) => assets.packed_ntsc_rgba8(),
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

impl FilterType {
    pub fn layout(self, source_logical_size: LogicalSize) -> FilterLayout {
        let logical_size = match self {
            FilterType::None => source_logical_size,
            FilterType::NtscRGB | FilterType::NtscComposite | FilterType::NtscSVideo => {
                LogicalSize {
                    width: nes_ntsc::NesNtsc::output_width(source_logical_size.width),
                    height: source_logical_size.height,
                }
            }
        };
        let physical_size = match self {
            FilterType::None => PhysicalSize {
                width: source_logical_size.width as f32 * 8.0 / 7.0,
                height: source_logical_size.height as f32,
            },
            FilterType::NtscRGB | FilterType::NtscComposite | FilterType::NtscSVideo => {
                PhysicalSize {
                    width: logical_size.width as f32,
                    height: source_logical_size.height as f32 * 2.0,
                }
            }
        };

        FilterLayout {
            source_logical_size,
            logical_size,
            physical_size,
        }
    }

    pub fn presentation(
        self,
        source_logical_size: LogicalSize,
        frame_format: VideoFrameFormat,
    ) -> VideoPresentation {
        let layout = self.layout(source_logical_size);
        let frame_spec = VideoFrameSpec::new(
            frame_format,
            layout.source_logical_size,
            layout.logical_size,
            layout.physical_size,
        );
        VideoPresentation::new(frame_spec)
    }

    pub fn rgba_presentation(self, source_logical_size: LogicalSize) -> VideoPresentation {
        self.presentation(source_logical_size, VideoFrameFormat::Rgba)
    }

    pub fn palette_presentation(self, source_logical_size: LogicalSize) -> VideoPresentation {
        self.presentation(source_logical_size, VideoFrameFormat::Palette)
    }

    fn encode_ntsc_packed_entries_rgba8(entries: &[u32]) -> Box<[u8]> {
        let color_count = PALETTE_TEXTURE_WIDTH as usize;
        let texture_height = NTSC_TEXTURE_HEIGHT as usize;
        let entry_stride = entries.len() / color_count;
        debug_assert!(entry_stride >= texture_height);

        let mut encoded = Vec::with_capacity(color_count * texture_height * 4);
        for row in 0..texture_height {
            for color in 0..color_count {
                encoded.extend_from_slice(&entries[color * entry_stride + row].to_be_bytes());
            }
        }
        encoded.into_boxed_slice()
    }

    pub(crate) fn palette_nes_video_assets(self) -> NesVideoAssets {
        let pipeline = match self {
            FilterType::None => VideoFilterPipeline::Palette {
                palette_rgba8: self.encoded_palette_rgba8(),
            },
            FilterType::NtscRGB | FilterType::NtscComposite | FilterType::NtscSVideo => {
                VideoFilterPipeline::Ntsc {
                    palette_rgba8: self.encoded_palette_rgba8(),
                    packed_ntsc_rgba8: self
                        .encoded_packed_ntsc_texture_rgba8()
                        .expect("NTSC filters should expose packed textures"),
                    split_ntsc_textures: self
                        .encoded_ntsc_textures_rgba8()
                        .expect("NTSC filters should expose split textures"),
                }
            }
        };

        NesVideoAssets::new(pipeline)
    }

    pub fn palette_console_video_assets(self) -> ConsoleVideoAssets {
        ConsoleVideoAssets::Nes(self.palette_nes_video_assets())
    }

    pub fn palette(self) -> [RGB; 64] {
        filters::rgb::PALETTE.map(RGB::from)
    }

    pub fn encoded_palette_rgba8(self) -> Box<[u8]> {
        self.palette()
            .into_iter()
            .flat_map(|color| [color.red, color.green, color.blue, u8::MAX])
            .collect::<Vec<_>>()
            .into_boxed_slice()
    }

    pub fn ntsc_setup(self) -> Option<Setup> {
        match self {
            FilterType::None => None,
            FilterType::NtscRGB => Some(Setup::RGB),
            FilterType::NtscComposite => Some(Setup::Composite),
            FilterType::NtscSVideo => Some(Setup::SVideo),
        }
    }

    pub fn shader_kernel_entries(self) -> Option<Box<[ShaderKernelEntry]>> {
        self.ntsc_setup()
            .map(|setup| nes_ntsc::NesNtsc::shader_kernel_entries(&setup))
    }

    pub fn packed_kernel_entries(self) -> Option<Box<[u32]>> {
        self.ntsc_setup()
            .map(|setup| nes_ntsc::NesNtsc::packed_kernel_entries(&setup))
    }

    pub fn encoded_packed_ntsc_texture_rgba8(self) -> Option<EncodedPackedNtscTexture> {
        self.packed_kernel_entries()
            .map(|entries| EncodedPackedNtscTexture {
                rgba8: Self::encode_ntsc_packed_entries_rgba8(entries.as_ref()),
            })
    }

    pub fn encoded_ntsc_textures_rgba8(self) -> Option<EncodedNtscTextures> {
        self.shader_kernel_entries().map(|entries| {
            let color_count = PALETTE_TEXTURE_WIDTH as usize;
            let texture_height = NTSC_TEXTURE_HEIGHT as usize;
            let entry_stride = entries.len() / color_count;
            debug_assert!(entry_stride >= texture_height);

            let mut primary = Vec::with_capacity(color_count * texture_height * 4);
            let mut secondary = Vec::with_capacity(color_count * texture_height * 4);
            for row in 0..texture_height {
                for color in 0..color_count {
                    let entry = entries[color * entry_stride + row];
                    let red = ((i32::from(entry.red)) + i32::from(i16::MAX) + 1) as u16;
                    let green = ((i32::from(entry.green)) + i32::from(i16::MAX) + 1) as u16;
                    let blue = ((i32::from(entry.blue)) + i32::from(i16::MAX) + 1) as u16;
                    primary.extend_from_slice(&[
                        (red >> 8) as u8,
                        red as u8,
                        (green >> 8) as u8,
                        green as u8,
                    ]);
                    secondary.extend_from_slice(&[(blue >> 8) as u8, blue as u8, 0, 0]);
                }
            }
            EncodedNtscTextures {
                primary_rgba8: primary.into_boxed_slice(),
                secondary_rgba8: secondary.into_boxed_slice(),
            }
        })
    }
}
