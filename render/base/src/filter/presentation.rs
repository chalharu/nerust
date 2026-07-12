use nerust_render_ntsc::{ShaderKernelEntry, setup::Setup};

use super::{FilterType, NTSC_TEXTURE_HEIGHT, PALETTE_TEXTURE_WIDTH, filters};
use crate::{LogicalSize, PhysicalSize, RGB, VideoFrameFormat, VideoFrameSpec, VideoPresentation};

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
pub struct PaletteAssets {
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
    Nes(PaletteAssets),
    // Future: Snes(SnesVideoAssets),
}

impl ConsoleVideoAssets {
    /// Return the inner [`PaletteAssets`] if this is the NES variant.
    pub fn as_nes(&self) -> Option<&PaletteAssets> {
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

impl PaletteAssets {
    pub(crate) fn new(pipeline: VideoFilterPipeline) -> Self {
        Self { pipeline }
    }

    pub fn pipeline_kind(&self) -> VideoPresentationPipelineKind {
        self.pipeline.kind()
    }

    fn is_palette_pipeline(&self) -> bool {
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
                    width: nerust_render_ntsc::Engine::output_width(source_logical_size.width),
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

    pub(crate) fn palette_assets(self) -> PaletteAssets {
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

        PaletteAssets::new(pipeline)
    }

    pub fn palette_console_video_assets(self) -> ConsoleVideoAssets {
        ConsoleVideoAssets::Nes(self.palette_assets())
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
            .map(|setup| nerust_render_ntsc::Engine::shader_kernel_entries(&setup))
    }

    pub fn packed_kernel_entries(self) -> Option<Box<[u32]>> {
        self.ntsc_setup()
            .map(|setup| nerust_render_ntsc::Engine::packed_kernel_entries(&setup))
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

#[cfg(test)]
mod tests {
    use crate::filter::{
        BLACK_PALETTE_INDEX, FilterFunc, FilterType, NTSC_TEXTURE_HEIGHT, PALETTE_TEXTURE_WIDTH,
    };
    use super::{ConsoleVideoAssets, VideoPresentationPipelineKind};
    use crate::{LogicalSize, RGB, VideoFrameFormat};

    const NTSC_ROW_OFFSETS: [[usize; 6]; 7] = [
        [0, 19, 31, 7, 26, 38],
        [1, 20, 32, 8, 27, 39],
        [2, 14, 33, 9, 21, 40],
        [3, 15, 34, 10, 22, 41],
        [4, 16, 28, 11, 23, 35],
        [5, 17, 29, 12, 24, 36],
        [6, 18, 30, 13, 25, 37],
    ];
    const NTSC_SOURCE_OFFSETS: [[i32; 6]; 7] = [
        [1, -1, 0, -2, -4, -3],
        [1, -1, 0, -2, -4, -3],
        [1, 2, 0, -2, -1, -3],
        [1, 2, 0, -2, -1, -3],
        [1, 2, 3, -2, -1, 0],
        [1, 2, 3, -2, -1, 0],
        [1, 2, 3, -2, -1, 0],
    ];

    fn decode_u16(high: u8, low: u8) -> i16 {
        (((u16::from(high) << 8) | u16::from(low)) as i32).wrapping_sub(i32::from(i16::MAX) + 1)
            as i16
    }

    fn decode_u32(bytes: &[u8], offset: usize) -> u32 {
        u32::from_be_bytes(bytes[offset..offset + 4].try_into().expect("RGBA8 texel"))
    }

    #[derive(Default)]
    struct RgbaCollector {
        bytes: Vec<u8>,
    }

    impl FilterFunc for RgbaCollector {
        fn filter_func(&mut self, color: RGB) {
            self.bytes
                .extend_from_slice(&[color.red, color.green, color.blue, u8::MAX]);
        }
    }

    fn clamp_impl(io: u32) -> u32 {
        const NTSC_CLAMP_MASK: u32 = 0x300c03;
        const NTSC_CLAMP_ADD: u32 = 0x20280a02;

        let sub = (io >> 9) & NTSC_CLAMP_MASK;
        let clamp = NTSC_CLAMP_ADD.wrapping_sub(sub);
        (io | clamp) & clamp.wrapping_sub(sub)
    }

    fn rgb_out_impl(raw: u32) -> [u8; 4] {
        let rgb = ((raw >> 5) & 0x00ff0000) | ((raw >> 3) & 0x0000ff00) | ((raw >> 1) & 0x000000ff);
        [
            ((rgb >> 16) & 0xff) as u8,
            ((rgb >> 8) & 0xff) as u8,
            (rgb & 0xff) as u8,
            u8::MAX,
        ]
    }

    fn collect_cpu_rgba(filter: FilterType, source: LogicalSize, source_frame: &[u8]) -> Vec<u8> {
        let mut filter_impl = filter.generate(source);
        let logical_size = filter_impl.logical_size();
        let mut collector = RgbaCollector {
            bytes: Vec::with_capacity(logical_size.width * logical_size.height * 4),
        };

        for &value in source_frame {
            filter_impl.push(value, &mut collector);
        }

        assert_eq!(
            collector.bytes.len(),
            logical_size.width * logical_size.height * 4
        );
        collector.bytes
    }

    fn palette_index(source_frame: &[u8], source: LogicalSize, x: i32, y: usize) -> u8 {
        if x < 0 || x >= source.width as i32 {
            return BLACK_PALETTE_INDEX;
        }
        source_frame[y * source.width + x as usize]
    }

    fn simulate_gpu_ntsc_rgba(
        filter: FilterType,
        source: LogicalSize,
        source_frame: &[u8],
    ) -> Vec<u8> {
        let packed_entries = filter
            .packed_kernel_entries()
            .expect("NTSC filters should expose packed entries");
        let logical_size = filter.layout(source).logical_size;
        let entry_stride = packed_entries.len() / PALETTE_TEXTURE_WIDTH as usize;
        let mut output = vec![0; logical_size.width * logical_size.height * 4];

        for y in 0..logical_size.height {
            let phase_row = (y % 3) * 42;
            for x in 0..logical_size.width {
                let chunk = x / 7;
                let sample = x - chunk * 7;
                let base = (chunk * 3) as i32;
                let row_offsets = NTSC_ROW_OFFSETS[sample];
                let source_offsets = NTSC_SOURCE_OFFSETS[sample];
                let mut sum = 0_u32;

                for (source_offset, row_offset) in source_offsets.into_iter().zip(row_offsets) {
                    let color = palette_index(source_frame, source, base + source_offset, y);
                    sum = sum.wrapping_add(
                        packed_entries[usize::from(color) * entry_stride + phase_row + row_offset],
                    );
                }

                let offset = (y * logical_size.width + x) * 4;
                output[offset..offset + 4].copy_from_slice(&rgb_out_impl(clamp_impl(sum)));
            }
        }

        output
    }

    #[test]
    fn encoded_ntsc_textures_are_row_major_and_complete() {
        let entries = FilterType::NtscComposite
            .shader_kernel_entries()
            .expect("NTSC filters should expose shader entries");
        let textures = FilterType::NtscComposite
            .encoded_ntsc_textures_rgba8()
            .expect("NTSC filters should expose encoded textures");
        let color_count = PALETTE_TEXTURE_WIDTH as usize;
        let texture_height = NTSC_TEXTURE_HEIGHT as usize;
        let entry_stride = entries.len() / color_count;

        assert_eq!(
            textures.primary_rgba8.len(),
            color_count * texture_height * 4
        );
        assert_eq!(
            textures.secondary_rgba8.len(),
            color_count * texture_height * 4
        );

        for (row, color) in [
            (0, 0),
            (0, color_count - 1),
            (1, 1),
            (texture_height - 1, 0),
        ] {
            let entry = entries[color * entry_stride + row];
            let offset = (row * color_count + color) * 4;
            assert_eq!(
                decode_u16(
                    textures.primary_rgba8[offset],
                    textures.primary_rgba8[offset + 1]
                ),
                entry.red
            );
            assert_eq!(
                decode_u16(
                    textures.primary_rgba8[offset + 2],
                    textures.primary_rgba8[offset + 3]
                ),
                entry.green
            );
            assert_eq!(
                decode_u16(
                    textures.secondary_rgba8[offset],
                    textures.secondary_rgba8[offset + 1]
                ),
                entry.blue
            );
        }
    }

    #[test]
    fn encoded_packed_ntsc_texture_is_row_major_big_endian_and_complete() {
        let entries = FilterType::NtscComposite
            .packed_kernel_entries()
            .expect("NTSC filters should expose packed entries");
        let texture = FilterType::NtscComposite
            .encoded_packed_ntsc_texture_rgba8()
            .expect("NTSC filters should expose packed textures");
        let color_count = PALETTE_TEXTURE_WIDTH as usize;
        let texture_height = NTSC_TEXTURE_HEIGHT as usize;
        let entry_stride = entries.len() / color_count;

        assert_eq!(texture.rgba8.len(), color_count * texture_height * 4);

        for (row, color) in [
            (0, 0),
            (0, color_count - 1),
            (1, 1),
            (texture_height / 2, color_count / 2),
            (texture_height - 1, 0),
        ] {
            let offset = (row * color_count + color) * 4;
            assert_eq!(
                decode_u32(texture.rgba8.as_ref(), offset),
                entries[color * entry_stride + row]
            );
        }
    }

    #[test]
    fn palette_presentation_uses_palette_pipeline() {
        let presentation = FilterType::None.palette_presentation(LogicalSize {
            width: 256,
            height: 240,
        });
        let assets = FilterType::None.palette_assets();

        assert_eq!(presentation.frame_format(), VideoFrameFormat::Palette);
        assert_eq!(
            assets.pipeline_kind(),
            VideoPresentationPipelineKind::Palette
        );
        assert!(!assets.palette_rgba8().is_empty());
        assert!(assets.packed_ntsc_rgba8().is_none());
        assert!(assets.split_ntsc_textures().is_none());
    }

    #[test]
    fn ntsc_presentation_exposes_both_ntsc_asset_formats() {
        let presentation = FilterType::NtscComposite.palette_presentation(LogicalSize {
            width: 256,
            height: 240,
        });
        let assets = FilterType::NtscComposite.palette_assets();

        assert_eq!(presentation.frame_format(), VideoFrameFormat::Palette);
        assert_eq!(assets.pipeline_kind(), VideoPresentationPipelineKind::Ntsc);
        assert!(!assets.palette_rgba8().is_empty());
        assert!(assets.packed_ntsc_rgba8().is_some());
        assert!(assets.split_ntsc_textures().is_some());
    }

    #[test]
    fn palette_console_video_assets_wrap_nes_pipeline() {
        let assets = FilterType::NtscComposite.palette_console_video_assets();

        match assets {
            ConsoleVideoAssets::Nes(nes_assets) => {
                assert_eq!(
                    nes_assets.pipeline_kind(),
                    VideoPresentationPipelineKind::Ntsc
                );
                assert!(nes_assets.uses_ntsc_pipeline());
            }
        }
    }

    #[test]
    fn rgba_presentation_uses_direct_pipeline() {
        let presentation = FilterType::NtscComposite.rgba_presentation(LogicalSize {
            width: 256,
            height: 240,
        });

        assert_eq!(presentation.frame_format(), VideoFrameFormat::Rgba);
    }

    #[test]
    fn nes_video_assets_some_for_palette_format() {
        let assets = FilterType::NtscComposite.palette_assets();
        assert!(assets.uses_ntsc_pipeline());
    }

    #[test]
    fn ntsc_gpu_reference_matches_cpu_filter_output() {
        let source = LogicalSize {
            width: 256,
            height: 3,
        };
        let source_frame = (0..source.height)
            .flat_map(|y| {
                (0..source.width).map(move |x| ((x * 11 + y * 17 + x * y * 5) % 64) as u8)
            })
            .collect::<Vec<_>>();

        for filter in [
            FilterType::NtscRGB,
            FilterType::NtscComposite,
            FilterType::NtscSVideo,
        ] {
            let cpu_output = collect_cpu_rgba(filter, source, &source_frame);
            let gpu_output = simulate_gpu_ntsc_rgba(filter, source, &source_frame);
            assert_eq!(
                gpu_output, cpu_output,
                "{filter:?} GPU reference diverged from CPU filter output"
            );
        }
    }
}
