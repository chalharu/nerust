use crate::filters;
use crate::presentation::{ConsoleVideoAssets, NesVideoAssets, VideoFilterPipeline};
use nerust_screen_traits::{
    VideoFrameFormat, VideoFrameSpec, VideoPresentation, logical_size::LogicalSize,
    physical_size::PhysicalSize, rgb::RGB,
};
use nes_ntsc::{ShaderKernelEntry, setup::Setup};

pub const BLACK_PALETTE_INDEX: u8 = nes_ntsc::BLACK;
pub const PALETTE_TEXTURE_WIDTH: u32 = 64;
pub const NTSC_TEXTURE_WIDTH: u32 = nes_ntsc::SHADER_COLOR_COUNT as u32;
pub const NTSC_TEXTURE_HEIGHT: u32 =
    (nes_ntsc::SHADER_PHASE_COUNT * nes_ntsc::SHADER_PHASE_ENTRY_COUNT) as u32;

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

pub trait NesFilter: Send {
    fn push(&mut self, value: u8, filter_func: &mut dyn FilterFunc);

    fn logical_size(&self) -> LogicalSize;
    fn physical_size(&self) -> PhysicalSize;
}

pub trait FilterFunc {
    fn filter_func(&mut self, value: RGB);
}

impl<F: filters::FilterUnit<Input = u8, Output = RGB>> NesFilter for F {
    fn push(&mut self, value: u8, filter_func: &mut dyn FilterFunc) {
        filters::FilterUnit::push(self, value, &mut |x| filter_func.filter_func(x))
    }

    fn logical_size(&self) -> LogicalSize {
        filters::FilterUnit::logical_size(self)
    }

    fn physical_size(&self) -> PhysicalSize {
        filters::FilterUnit::physical_size(self)
    }
}

#[derive(Debug, Clone, Copy)]
pub enum FilterType {
    None,
    NtscRGB,
    NtscComposite,
    NtscSVideo,
}

impl FilterType {
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

    pub fn generate(self, size: LogicalSize) -> Box<dyn NesFilter> {
        match self {
            FilterType::None => Box::new(filters::rgb::NesRgb::new(size)),
            FilterType::NtscRGB => Box::new(filters::ntsc::NesNtsc::rgb(size)),
            FilterType::NtscComposite => Box::new(filters::ntsc::NesNtsc::composite(size)),
            FilterType::NtscSVideo => Box::new(filters::ntsc::NesNtsc::svideo(size)),
        }
    }

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
