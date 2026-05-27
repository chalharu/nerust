// Copyright (c) 2026 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use crate::media::{SCREEN_HEIGHT, SCREEN_WIDTH};
use nerust_snes_core::Core;

use super::{
    RenderError, VISIBLE_BG_Y_OFFSET,
    color::{cgram_color_rgba, put_pixel},
    mode7::render_mode7_bg1,
    tile::{bg_chr_2bpp_pixel, bg_chr_8bpp_pixel, chr_4bpp_pixel, read_tilemap_entry},
};

const MODE0_BG1_CGRAM_BASE: usize = 0;

pub(super) fn render_bg1(core: &Core, brightness: u8, rgba: &mut [u8]) -> Result<(), RenderError> {
    let bgmode = core.peek(0x002105);
    let mode = bgmode & 0x07;
    let mode = Bg1RenderMode::from_bgmode(mode)?;
    if mode == Bg1RenderMode::Mode7 {
        render_mode7_bg1(core, brightness, rgba);
        return Ok(());
    }

    let bg1sc = core.peek(0x002107);
    let bg12nba = core.peek(0x00210B);
    let context = Bg1RenderContext {
        mode,
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
        Bg1RenderMode::Mode7 => unreachable!("Mode7 uses its own renderer"),
    };
    if color == 0 {
        return None;
    }

    let palette = usize::from((entry >> 10) & 0x07);
    let color_index = match context.mode {
        Bg1RenderMode::Mode0 => MODE0_BG1_CGRAM_BASE + palette * 4 + usize::from(color),
        Bg1RenderMode::Mode1 => palette * 16 + usize::from(color),
        Bg1RenderMode::Mode3 => usize::from(color),
        Bg1RenderMode::Mode7 => unreachable!("Mode7 uses its own renderer"),
    };
    Some(cgram_color_rgba(core, color_index, context.brightness))
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Bg1RenderMode {
    Mode0,
    Mode1,
    Mode3,
    Mode7,
}

impl Bg1RenderMode {
    fn from_bgmode(mode: u8) -> Result<Self, RenderError> {
        match mode {
            0 => Ok(Self::Mode0),
            1 => Ok(Self::Mode1),
            3 => Ok(Self::Mode3),
            7 => Ok(Self::Mode7),
            _ => Err(RenderError::UnsupportedBgMode { mode }),
        }
    }

    const fn tile_bytes(self) -> usize {
        match self {
            Self::Mode0 => 16,
            Self::Mode1 => 32,
            Self::Mode3 => 64,
            Self::Mode7 => 0,
        }
    }
}
