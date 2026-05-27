// Copyright (c) 2026 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use crate::media::{SCREEN_HEIGHT, SCREEN_WIDTH};
use nerust_snes_core::Core;

const MODE0_BG1_CGRAM_BASE: usize = 32;
const VISIBLE_BG_Y_OFFSET: usize = 1;

#[derive(Debug, thiserror::Error)]
pub enum RenderError {
    #[error(
        "unsupported BG mode {mode}; SNES rom_test currently supports BG1 rendering for modes 0, 1, and 3"
    )]
    UnsupportedBgMode { mode: u8 },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderedScreen {
    pub rgba: Vec<u8>,
}

pub fn render_screen(core: &Core) -> Result<RenderedScreen, RenderError> {
    let inidisp = core.peek(0x002100);
    let brightness = inidisp & 0x0F;
    let mut rgba = opaque_black_screen();

    if inidisp & 0x80 != 0 || brightness == 0 {
        return Ok(RenderedScreen { rgba });
    }

    let backdrop = cgram_color_rgba(core, 0, brightness);
    for pixel in rgba.chunks_exact_mut(4) {
        pixel.copy_from_slice(&backdrop);
    }

    let tm = core.peek(0x00212C);
    if tm & 0x01 != 0 {
        render_bg1(core, brightness, &mut rgba)?;
    }
    if tm & 0x10 != 0 {
        render_obj(core, brightness, &mut rgba);
    }

    Ok(RenderedScreen { rgba })
}

fn opaque_black_screen() -> Vec<u8> {
    let mut rgba = vec![0; SCREEN_WIDTH * SCREEN_HEIGHT * 4];
    for pixel in rgba.chunks_exact_mut(4) {
        pixel[3] = 0xFF;
    }
    rgba
}

fn render_bg1(core: &Core, brightness: u8, rgba: &mut [u8]) -> Result<(), RenderError> {
    let bgmode = core.peek(0x002105);
    let mode = bgmode & 0x07;
    let bg1sc = core.peek(0x002107);
    let bg12nba = core.peek(0x00210B);
    let context = Bg1RenderContext {
        mode: Bg1RenderMode::from_bgmode(mode)?,
        tilemap_base: (usize::from(bg1sc & 0xFC)) << 9,
        chr_base: usize::from(bg12nba & 0x0F) << 13,
        tile_size: if bgmode & 0x10 != 0 { 16 } else { 8 },
        tilemap_width_tiles: if bg1sc & 0x01 != 0 { 64 } else { 32 },
        brightness,
    };
    let tilemap_height_tiles = if bg1sc & 0x02 != 0 { 64 } else { 32 };
    let tilemap_width_pixels = context.tilemap_width_tiles * context.tile_size;
    let tilemap_height_pixels = tilemap_height_tiles * context.tile_size;
    let hofs = usize::from(core.bg1_hofs()) % tilemap_width_pixels.max(1);
    let vofs = (usize::from(core.bg1_vofs()) + VISIBLE_BG_Y_OFFSET) % tilemap_height_pixels.max(1);

    for screen_y in 0..SCREEN_HEIGHT {
        let bg_y = (screen_y + vofs) % tilemap_height_pixels;
        for screen_x in 0..SCREEN_WIDTH {
            let bg_x = (screen_x + hofs) % tilemap_width_pixels;
            if let Some(color) = bg1_pixel(core, &context, bg_x, bg_y) {
                put_pixel(rgba, screen_x, screen_y, color);
            }
        }
    }

    Ok(())
}

fn bg1_pixel(core: &Core, context: &Bg1RenderContext, bg_x: usize, bg_y: usize) -> Option<[u8; 4]> {
    let tile_x = bg_x / context.tile_size;
    let tile_y = bg_y / context.tile_size;
    let entry = read_tilemap_entry(
        core,
        context.tilemap_base,
        context.tilemap_width_tiles,
        tile_x,
        tile_y,
    );

    let mut tile_pixel_x = bg_x % context.tile_size;
    let mut tile_pixel_y = bg_y % context.tile_size;
    if entry & 0x4000 != 0 {
        tile_pixel_x = context.tile_size - 1 - tile_pixel_x;
    }
    if entry & 0x8000 != 0 {
        tile_pixel_y = context.tile_size - 1 - tile_pixel_y;
    }

    let subtile_x = tile_pixel_x / 8;
    let subtile_y = tile_pixel_y / 8;
    let pixel_x = tile_pixel_x % 8;
    let pixel_y = tile_pixel_y % 8;
    let tile_number = usize::from(entry & 0x03FF) + subtile_x + subtile_y * 16;
    let tile_addr = context.chr_base + tile_number * context.mode.tile_bytes();
    let color = match context.mode {
        Bg1RenderMode::Mode0 => bg_chr_2bpp_pixel(core, tile_addr, pixel_x, pixel_y),
        Bg1RenderMode::Mode1 => chr_4bpp_pixel(core, tile_addr, pixel_x, pixel_y),
        Bg1RenderMode::Mode3 => bg_chr_8bpp_pixel(core, tile_addr, pixel_x, pixel_y),
    };
    if color == 0 {
        return None;
    }

    let palette = usize::from((entry >> 10) & 0x07);
    let color_index = match context.mode {
        Bg1RenderMode::Mode0 => MODE0_BG1_CGRAM_BASE + palette * 4 + usize::from(color),
        Bg1RenderMode::Mode1 => palette * 16 + usize::from(color),
        Bg1RenderMode::Mode3 => usize::from(color),
    };
    Some(cgram_color_rgba(core, color_index, context.brightness))
}

fn render_obj(core: &Core, brightness: u8, rgba: &mut [u8]) {
    let obsel = core.peek(0x002101);
    let (small_size, large_size) = obj_size_pair((obsel >> 5) & 0x07);

    for sprite_index in (0..128).rev() {
        let base = sprite_index * 4;
        let x_low = core.peek_oam(base);
        let y = core.peek_oam(base + 1);
        let tile = core.peek_oam(base + 2);
        let attributes = core.peek_oam(base + 3);
        let extra = core.peek_oam(512 + sprite_index / 4);
        let pair_shift = (sprite_index % 4) * 2;
        let x_high = (extra >> pair_shift) & 0x01 != 0;
        let large = (extra >> (pair_shift + 1)) & 0x01 != 0;
        let size = if large { large_size } else { small_size };

        let x = if x_high {
            i16::from(x_low) - 256
        } else {
            i16::from(x_low)
        };
        let mut y = i16::from(y);
        if y >= SCREEN_HEIGHT as i16 {
            y -= 256;
        }

        for sprite_y in 0..size.height {
            let target_y = y + sprite_y as i16;
            if !(0..SCREEN_HEIGHT as i16).contains(&target_y) {
                continue;
            }

            let source_y = if attributes & 0x80 != 0 {
                size.height - 1 - sprite_y
            } else {
                sprite_y
            };
            let tile_row = usize::from(source_y / 8);
            let pixel_y = usize::from(source_y % 8);

            for sprite_x in 0..size.width {
                let target_x = x + sprite_x as i16;
                if !(0..SCREEN_WIDTH as i16).contains(&target_x) {
                    continue;
                }

                let source_x = if attributes & 0x40 != 0 {
                    size.width - 1 - sprite_x
                } else {
                    sprite_x
                };
                let tile_column = usize::from(source_x / 8);
                let pixel_x = usize::from(source_x % 8);
                let tile_number = (usize::from(tile) | (usize::from(attributes & 0x01) << 8))
                    + tile_column
                    + tile_row * 16;
                let tile_addr = obj_tile_address(obsel, tile_number);
                let color = chr_4bpp_pixel(core, tile_addr, pixel_x, pixel_y);
                if color == 0 {
                    continue;
                }

                let palette = usize::from((attributes >> 1) & 0x07);
                let color =
                    cgram_color_rgba(core, 128 + palette * 16 + usize::from(color), brightness);
                put_pixel(rgba, target_x as usize, target_y as usize, color);
            }
        }
    }
}

fn read_tilemap_entry(
    core: &Core,
    tilemap_base: usize,
    tilemap_width_tiles: usize,
    tile_x: usize,
    tile_y: usize,
) -> u16 {
    let quadrant_columns = tilemap_width_tiles / 32;
    let quadrant = (tile_y / 32) * quadrant_columns + (tile_x / 32);
    let quadrant_base = tilemap_base + quadrant * 2048;
    let entry_offset = ((tile_y % 32) * 32 + (tile_x % 32)) * 2;
    u16::from_le_bytes([
        core.peek_vram(quadrant_base + entry_offset),
        core.peek_vram(quadrant_base + entry_offset + 1),
    ])
}

#[derive(Debug, Clone, Copy)]
struct Bg1RenderContext {
    mode: Bg1RenderMode,
    tilemap_base: usize,
    chr_base: usize,
    tile_size: usize,
    tilemap_width_tiles: usize,
    brightness: u8,
}

#[derive(Debug, Clone, Copy)]
enum Bg1RenderMode {
    Mode0,
    Mode1,
    Mode3,
}

impl Bg1RenderMode {
    fn from_bgmode(mode: u8) -> Result<Self, RenderError> {
        match mode {
            0 => Ok(Self::Mode0),
            1 => Ok(Self::Mode1),
            3 => Ok(Self::Mode3),
            _ => Err(RenderError::UnsupportedBgMode { mode }),
        }
    }

    const fn tile_bytes(self) -> usize {
        match self {
            Self::Mode0 => 16,
            Self::Mode1 => 32,
            Self::Mode3 => 64,
        }
    }
}

fn cgram_color_rgba(core: &Core, color_index: usize, brightness: u8) -> [u8; 4] {
    let base = color_index * 2;
    let color = u16::from_le_bytes([core.peek_cgram(base), core.peek_cgram(base + 1)]) & 0x7FFF;
    snes_color_to_rgba(color, brightness)
}

fn snes_color_to_rgba(color: u16, brightness: u8) -> [u8; 4] {
    let red = scale_channel((color & 0x1F) as u8, brightness);
    let green = scale_channel(((color >> 5) & 0x1F) as u8, brightness);
    let blue = scale_channel(((color >> 10) & 0x1F) as u8, brightness);
    [red, green, blue, 0xFF]
}

fn scale_channel(channel: u8, brightness: u8) -> u8 {
    let expanded = (u16::from(channel) * 255 + 15) / 31;
    ((expanded * u16::from(brightness) + 7) / 15) as u8
}

fn put_pixel(rgba: &mut [u8], x: usize, y: usize, color: [u8; 4]) {
    let offset = (y * SCREEN_WIDTH + x) * 4;
    rgba[offset..offset + 4].copy_from_slice(&color);
}

#[cfg(test)]
fn decode_2bpp_pixel(tile: &[u8], x: usize, y: usize) -> u8 {
    let row = y * 2;
    let shift = 7 - x;
    let plane0 = (tile[row] >> shift) & 0x01;
    let plane1 = (tile[row + 1] >> shift) & 0x01;
    plane0 | (plane1 << 1)
}

fn bg_chr_2bpp_pixel(core: &Core, tile_addr: usize, x: usize, y: usize) -> u8 {
    let row = y * 2;
    let shift = 7 - x;
    let plane0 = (core.peek_vram(tile_addr + row) >> shift) & 0x01;
    let plane1 = (core.peek_vram(tile_addr + row + 1) >> shift) & 0x01;
    plane0 | (plane1 << 1)
}

#[cfg(test)]
fn decode_4bpp_pixel(tile: &[u8], x: usize, y: usize) -> u8 {
    let row = y * 2;
    let shift = 7 - x;
    let plane0 = (tile[row] >> shift) & 0x01;
    let plane1 = (tile[row + 1] >> shift) & 0x01;
    let plane2 = (tile[0x10 + row] >> shift) & 0x01;
    let plane3 = (tile[0x10 + row + 1] >> shift) & 0x01;
    plane0 | (plane1 << 1) | (plane2 << 2) | (plane3 << 3)
}

fn chr_4bpp_pixel(core: &Core, tile_addr: usize, x: usize, y: usize) -> u8 {
    let row = y * 2;
    let shift = 7 - x;
    let plane0 = (core.peek_vram(tile_addr + row) >> shift) & 0x01;
    let plane1 = (core.peek_vram(tile_addr + row + 1) >> shift) & 0x01;
    let plane2 = (core.peek_vram(tile_addr + 0x10 + row) >> shift) & 0x01;
    let plane3 = (core.peek_vram(tile_addr + 0x10 + row + 1) >> shift) & 0x01;
    plane0 | (plane1 << 1) | (plane2 << 2) | (plane3 << 3)
}

#[cfg(test)]
fn decode_8bpp_pixel(tile: &[u8], x: usize, y: usize) -> u8 {
    let row = y * 2;
    let shift = 7 - x;
    let plane0 = (tile[row] >> shift) & 0x01;
    let plane1 = (tile[row + 1] >> shift) & 0x01;
    let plane2 = (tile[0x10 + row] >> shift) & 0x01;
    let plane3 = (tile[0x10 + row + 1] >> shift) & 0x01;
    let plane4 = (tile[0x20 + row] >> shift) & 0x01;
    let plane5 = (tile[0x20 + row + 1] >> shift) & 0x01;
    let plane6 = (tile[0x30 + row] >> shift) & 0x01;
    let plane7 = (tile[0x30 + row + 1] >> shift) & 0x01;
    plane0
        | (plane1 << 1)
        | (plane2 << 2)
        | (plane3 << 3)
        | (plane4 << 4)
        | (plane5 << 5)
        | (plane6 << 6)
        | (plane7 << 7)
}

fn bg_chr_8bpp_pixel(core: &Core, tile_addr: usize, x: usize, y: usize) -> u8 {
    let row = y * 2;
    let shift = 7 - x;
    let plane0 = (core.peek_vram(tile_addr + row) >> shift) & 0x01;
    let plane1 = (core.peek_vram(tile_addr + row + 1) >> shift) & 0x01;
    let plane2 = (core.peek_vram(tile_addr + 0x10 + row) >> shift) & 0x01;
    let plane3 = (core.peek_vram(tile_addr + 0x10 + row + 1) >> shift) & 0x01;
    let plane4 = (core.peek_vram(tile_addr + 0x20 + row) >> shift) & 0x01;
    let plane5 = (core.peek_vram(tile_addr + 0x20 + row + 1) >> shift) & 0x01;
    let plane6 = (core.peek_vram(tile_addr + 0x30 + row) >> shift) & 0x01;
    let plane7 = (core.peek_vram(tile_addr + 0x30 + row + 1) >> shift) & 0x01;
    plane0
        | (plane1 << 1)
        | (plane2 << 2)
        | (plane3 << 3)
        | (plane4 << 4)
        | (plane5 << 5)
        | (plane6 << 6)
        | (plane7 << 7)
}

fn obj_tile_address(obsel: u8, tile_number: usize) -> usize {
    let base = usize::from(obsel & 0x07) * 0x4000;
    let gap = usize::from((obsel >> 3) & 0x03) * 0x2000;
    base + tile_number * 32 + ((tile_number >> 8) * gap)
}

fn obj_size_pair(size_select: u8) -> (ObjSize, ObjSize) {
    match size_select {
        0 => (ObjSize::new(8, 8), ObjSize::new(16, 16)),
        1 => (ObjSize::new(8, 8), ObjSize::new(32, 32)),
        2 => (ObjSize::new(8, 8), ObjSize::new(64, 64)),
        3 => (ObjSize::new(16, 16), ObjSize::new(32, 32)),
        4 => (ObjSize::new(16, 16), ObjSize::new(64, 64)),
        5 => (ObjSize::new(32, 32), ObjSize::new(64, 64)),
        6 => (ObjSize::new(16, 32), ObjSize::new(32, 64)),
        _ => (ObjSize::new(16, 32), ObjSize::new(32, 32)),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ObjSize {
    width: u8,
    height: u8,
}

impl ObjSize {
    const fn new(width: u8, height: u8) -> Self {
        Self { width, height }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        decode_2bpp_pixel, decode_4bpp_pixel, decode_8bpp_pixel, obj_tile_address,
        opaque_black_screen, render_screen, scale_channel,
    };
    use nerust_snes_core::Core;

    const HEADER_OFFSET: usize = 0x7FC0;
    const RESET_VECTOR_OFFSET: usize = 0x7FFC;

    fn build_lorom(reset_vector: u16) -> Vec<u8> {
        let mut rom = vec![0; 0x10000];
        rom[HEADER_OFFSET..HEADER_OFFSET + 21].copy_from_slice(b"TEST SCREEN ROM      ");
        rom[0x7FD5] = 0x30;
        rom[0x7FD7] = 0x08;
        rom[RESET_VECTOR_OFFSET..RESET_VECTOR_OFFSET + 2]
            .copy_from_slice(&reset_vector.to_le_bytes());
        rom
    }

    #[test]
    fn decode_2bpp_pixel_reads_planar_tile_bits() {
        let tile = [
            0b0101_0101,
            0b0011_0011,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
        ];

        assert_eq!(decode_2bpp_pixel(&tile, 0, 0), 0);
        assert_eq!(decode_2bpp_pixel(&tile, 1, 0), 1);
        assert_eq!(decode_2bpp_pixel(&tile, 3, 0), 3);
        assert_eq!(decode_2bpp_pixel(&tile, 4, 0), 0);
    }

    #[test]
    fn decode_4bpp_pixel_reads_all_four_bitplanes() {
        let mut tile = [0; 32];
        tile[0] = 0b1000_0000;
        tile[0x10] = 0b1000_0000;
        tile[0x11] = 0b1000_0000;

        assert_eq!(decode_4bpp_pixel(&tile, 0, 0), 0b1101);
    }

    #[test]
    fn decode_8bpp_pixel_reads_all_eight_bitplanes() {
        let mut tile = [0; 64];
        tile[0] = 0b1000_0000;
        tile[0x10] = 0b1000_0000;
        tile[0x20] = 0b1000_0000;
        tile[0x30] = 0b1000_0000;

        assert_eq!(decode_8bpp_pixel(&tile, 0, 0), 0b0101_0101);
    }

    #[test]
    fn obj_tile_address_applies_gap_to_secondary_page() {
        assert_eq!(obj_tile_address(0b0000_1000, 0x00FF), 0x1FE0);
        assert_eq!(obj_tile_address(0b0000_1000, 0x0100), 0x4000);
    }

    #[test]
    fn brightness_scaling_reaches_black_and_full_intensity() {
        assert_eq!(scale_channel(0x1F, 0x00), 0x00);
        assert_eq!(scale_channel(0x1F, 0x0F), 0xFF);
    }

    #[test]
    fn opaque_black_screen_uses_opaque_pixels() {
        let rgba = opaque_black_screen();

        assert_eq!(&rgba[..4], &[0x00, 0x00, 0x00, 0xFF]);
        assert_eq!(&rgba[rgba.len() - 4..], &[0x00, 0x00, 0x00, 0xFF]);
    }

    #[test]
    fn brightness_zero_renders_opaque_black_frame() {
        let core = Core::from_rom_bytes(&build_lorom(0x8000)).unwrap();

        let rendered = render_screen(&core).unwrap();

        assert_eq!(&rendered.rgba[..4], &[0x00, 0x00, 0x00, 0xFF]);
        assert_eq!(
            &rendered.rgba[rendered.rgba.len() - 4..],
            &[0x00, 0x00, 0x00, 0xFF]
        );
    }
}
