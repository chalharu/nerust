// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

mod filters;
pub mod presentation;
use nerust_screen_logical::LogicalSize;
use nerust_screen_physical::PhysicalSize;
use nerust_screen_rgb::RGB;

pub const BLACK_PALETTE_INDEX: u8 = nes_ntsc::BLACK;
pub const PALETTE_TEXTURE_WIDTH: u32 = 64;
pub const NTSC_TEXTURE_WIDTH: u32 = nes_ntsc::SHADER_COLOR_COUNT as u32;
pub const NTSC_TEXTURE_HEIGHT: u32 =
    (nes_ntsc::SHADER_PHASE_COUNT * nes_ntsc::SHADER_PHASE_ENTRY_COUNT) as u32;

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
    pub fn generate(self, size: LogicalSize) -> Box<dyn NesFilter> {
        match self {
            FilterType::None => Box::new(filters::rgb::NesRgb::new(size)),
            FilterType::NtscRGB => Box::new(filters::ntsc::NesNtsc::rgb(size)),
            FilterType::NtscComposite => Box::new(filters::ntsc::NesNtsc::composite(size)),
            FilterType::NtscSVideo => Box::new(filters::ntsc::NesNtsc::svideo(size)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        BLACK_PALETTE_INDEX, FilterFunc, FilterType, NTSC_TEXTURE_HEIGHT, PALETTE_TEXTURE_WIDTH,
        presentation::{ConsoleVideoAssets, VideoPresentationPipelineKind},
    };
    use nerust_screen_logical::LogicalSize;
    use nerust_screen_rgb::RGB;
    use nerust_screen_video::VideoFrameFormat;

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
        let assets = FilterType::None.palette_nes_video_assets();

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
        let assets = FilterType::NtscComposite.palette_nes_video_assets();

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
        let assets = FilterType::NtscComposite.palette_nes_video_assets();
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
