// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

mod filters;

use nerust_screen_traits::{LogicalSize, PhysicalSize, RGB};
use nes_ntsc::{Setup, ShaderKernelEntry};

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

pub struct EncodedNtscTextures {
    pub primary_rgba8: Box<[u8]>,
    pub secondary_rgba8: Box<[u8]>,
}

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

#[cfg(test)]
mod tests {
    use super::{FilterType, NTSC_TEXTURE_HEIGHT, PALETTE_TEXTURE_WIDTH};

    fn decode_u16(high: u8, low: u8) -> i16 {
        (((u16::from(high) << 8) | u16::from(low)) as i32).wrapping_sub(i32::from(i16::MAX) + 1)
            as i16
    }

    fn decode_u32(bytes: &[u8], offset: usize) -> u32 {
        u32::from_be_bytes(bytes[offset..offset + 4].try_into().expect("RGBA8 texel"))
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
}
