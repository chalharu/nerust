// Copyright (c) 2026 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use crate::{SCREEN_HEIGHT, SCREEN_WIDTH};
use nerust_snes_core::Core;

use super::{
    BgLayer, RenderError, VISIBLE_BG_Y_OFFSET,
    color::{cgram_color_rgba, put_pixel},
    main_screen_for_line,
    mode7::render_mode7_bg1,
    presented_bg_line,
    tile::{bg_chr_2bpp_pixel, bg_chr_8bpp_pixel, chr_4bpp_pixel, read_tilemap_entry},
    use_presented_bg_scroll,
};

pub(super) fn render_bg1(
    core: &Core,
    layer: BgLayer,
    brightness: u8,
    current_tm: u8,
    use_presented_tm: bool,
    rgba: &mut [u8],
) -> Result<(), RenderError> {
    if !screen_uses_layer(core, layer, current_tm, use_presented_tm) {
        return Ok(());
    }

    let bgmode = core.peek(0x002105);
    let screen_mode = bgmode & 0x07;
    let Some(mode) = BgRenderMode::from_bgmode(layer, screen_mode)? else {
        return Ok(());
    };
    if mode == BgRenderMode::Mode7 {
        render_mode7_bg1(core, brightness, current_tm, use_presented_tm, rgba);
        return Ok(());
    }

    let bgsc = core.peek(layer.bgsc_register());
    let bg12nba = core.peek(0x00210B);
    let bg34nba = core.peek(0x00210C);
    let context = Bg1RenderContext {
        mode,
        tilemap_base: (usize::from(bgsc & 0xFC)) << 9,
        chr_base: layer.chr_base(bg12nba, bg34nba),
        tile_size: if bgmode & layer.tile_size_mask() != 0 {
            16
        } else {
            8
        },
        horizontal_scale: if screen_mode == 5 || screen_mode == 6 {
            2
        } else {
            1
        },
        tilemap_width_tiles: if bgsc & 0x01 != 0 { 64 } else { 32 },
        bpp2_palette_base: bpp2_palette_base(layer, screen_mode),
    };
    let palette = cgram_palette_rgba(core, brightness);
    let tilemap_height_tiles = if bgsc & 0x02 != 0 { 64 } else { 32 };
    let tilemap_width_pixels = context.tilemap_width_tiles * context.tile_size;
    let tilemap_height_pixels = tilemap_height_tiles * context.tile_size;
    let (current_hofs, current_vofs) = layer.current_scroll(core);
    let use_presented_scroll = use_presented_bg_scroll(core, layer);

    for screen_y in 0..SCREEN_HEIGHT {
        if main_screen_for_line(core, screen_y, current_tm, use_presented_tm) & layer.tm_mask() == 0
        {
            continue;
        }
        let presented = use_presented_scroll
            .then(|| presented_bg_line(core, layer, screen_y))
            .flatten();
        let hofs = presented.map_or(usize::from(current_hofs), |line| usize::from(line.hofs))
            % tilemap_width_pixels.max(1);
        let vofs = (presented.map_or(usize::from(current_vofs), |line| usize::from(line.vofs))
            + VISIBLE_BG_Y_OFFSET)
            % tilemap_height_pixels.max(1);
        let bg_y = (screen_y + vofs) % tilemap_height_pixels;
        for screen_x in 0..SCREEN_WIDTH {
            let bg_x = (screen_x * context.horizontal_scale + hofs) % tilemap_width_pixels;
            if let Some(color) = bg1_pixel(core, &context, &palette, bg_x, bg_y) {
                put_pixel(rgba, screen_x, screen_y, color);
            }
        }
    }

    Ok(())
}

fn screen_uses_layer(core: &Core, layer: BgLayer, current_tm: u8, use_presented_tm: bool) -> bool {
    if !use_presented_tm {
        return current_tm & layer.tm_mask() != 0;
    }

    (0..SCREEN_HEIGHT).any(|screen_y| {
        main_screen_for_line(core, screen_y, current_tm, use_presented_tm) & layer.tm_mask() != 0
    })
}

fn cgram_palette_rgba(core: &Core, brightness: u8) -> [[u8; 4]; 256] {
    std::array::from_fn(|index| cgram_color_rgba(core, index, brightness))
}

fn bg1_pixel(
    core: &Core,
    context: &Bg1RenderContext,
    palette: &[[u8; 4]; 256],
    bg_x: usize,
    bg_y: usize,
) -> Option<[u8; 4]> {
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
        BgRenderMode::Bpp2 => bg_chr_2bpp_pixel(core, tile_addr, pixel_x, pixel_y),
        BgRenderMode::Bpp4 => chr_4bpp_pixel(core, tile_addr, pixel_x, pixel_y),
        BgRenderMode::Bpp8 => bg_chr_8bpp_pixel(core, tile_addr, pixel_x, pixel_y),
        BgRenderMode::Mode7 => unreachable!("Mode7 uses its own renderer"),
    };
    if color == 0 {
        return None;
    }

    let tile_palette = usize::from((entry >> 10) & 0x07);
    let color_index = match context.mode {
        BgRenderMode::Bpp2 => context.bpp2_palette_base + tile_palette * 4 + usize::from(color),
        BgRenderMode::Bpp4 => tile_palette * 16 + usize::from(color),
        BgRenderMode::Bpp8 => usize::from(color),
        BgRenderMode::Mode7 => unreachable!("Mode7 uses its own renderer"),
    };
    Some(palette[color_index])
}

#[derive(Debug, Clone, Copy)]
struct Bg1RenderContext {
    mode: BgRenderMode,
    tilemap_base: usize,
    chr_base: usize,
    tile_size: usize,
    horizontal_scale: usize,
    tilemap_width_tiles: usize,
    bpp2_palette_base: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BgRenderMode {
    Bpp2,
    Bpp4,
    Bpp8,
    Mode7,
}

impl BgRenderMode {
    fn from_bgmode(layer: BgLayer, mode: u8) -> Result<Option<Self>, RenderError> {
        match (layer, mode) {
            (BgLayer::Bg1 | BgLayer::Bg2 | BgLayer::Bg3 | BgLayer::Bg4, 0) => Ok(Some(Self::Bpp2)),
            (BgLayer::Bg1 | BgLayer::Bg2, 1) => Ok(Some(Self::Bpp4)),
            (BgLayer::Bg3, 1) => Ok(Some(Self::Bpp2)),
            (BgLayer::Bg4, 1) => Ok(None),
            (BgLayer::Bg1 | BgLayer::Bg2, 2) => Ok(Some(Self::Bpp4)),
            (BgLayer::Bg1, 3) => Ok(Some(Self::Bpp8)),
            (BgLayer::Bg2, 3) => Ok(Some(Self::Bpp4)),
            (BgLayer::Bg1, 4) => Ok(Some(Self::Bpp8)),
            (BgLayer::Bg2, 4) => Ok(Some(Self::Bpp2)),
            (BgLayer::Bg1, 5 | 6) => Ok(Some(Self::Bpp4)),
            (BgLayer::Bg2, 5) => Ok(Some(Self::Bpp2)),
            (BgLayer::Bg1, 7) => Ok(Some(Self::Mode7)),
            (_, 2..=7) => Ok(None),
            _ => Err(RenderError::UnsupportedBgMode { mode }),
        }
    }

    const fn tile_bytes(self) -> usize {
        match self {
            Self::Bpp2 => 16,
            Self::Bpp4 => 32,
            Self::Bpp8 => 64,
            Self::Mode7 => 0,
        }
    }
}

fn bpp2_palette_base(layer: BgLayer, screen_mode: u8) -> usize {
    match screen_mode {
        0 => layer.mode0_palette_base(),
        1 if layer == BgLayer::Bg3 => 0,
        _ => 0,
    }
}

impl BgLayer {
    const fn bgsc_register(self) -> u32 {
        match self {
            Self::Bg1 => 0x002107,
            Self::Bg2 => 0x002108,
            Self::Bg3 => 0x002109,
            Self::Bg4 => 0x00210A,
        }
    }

    const fn tile_size_mask(self) -> u8 {
        match self {
            Self::Bg1 => 0x10,
            Self::Bg2 => 0x20,
            Self::Bg3 => 0x40,
            Self::Bg4 => 0x80,
        }
    }

    const fn chr_base(self, bg12nba: u8, bg34nba: u8) -> usize {
        match self {
            Self::Bg1 => ((bg12nba & 0x0F) as usize) << 13,
            Self::Bg2 => ((bg12nba >> 4) as usize) << 13,
            Self::Bg3 => ((bg34nba & 0x0F) as usize) << 13,
            Self::Bg4 => ((bg34nba >> 4) as usize) << 13,
        }
    }

    fn current_scroll(self, core: &Core) -> (u16, u16) {
        match self {
            Self::Bg1 => (core.bg1_hofs(), core.bg1_vofs()),
            Self::Bg2 => (core.bg2_hofs(), core.bg2_vofs()),
            Self::Bg3 => (core.bg3_hofs(), core.bg3_vofs()),
            Self::Bg4 => (core.bg4_hofs(), core.bg4_vofs()),
        }
    }

    const fn mode0_palette_base(self) -> usize {
        match self {
            Self::Bg1 => 0,
            Self::Bg2 => 32,
            Self::Bg3 => 64,
            Self::Bg4 => 96,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{BgLayer, BgRenderMode, bpp2_palette_base};

    #[test]
    fn bpp2_palette_base_uses_mode0_layer_blocks() {
        assert_eq!(bpp2_palette_base(BgLayer::Bg1, 0), 0);
        assert_eq!(bpp2_palette_base(BgLayer::Bg2, 0), 32);
        assert_eq!(bpp2_palette_base(BgLayer::Bg3, 0), 64);
        assert_eq!(bpp2_palette_base(BgLayer::Bg4, 0), 96);
    }

    #[test]
    fn mode1_bg3_uses_first_palette_block() {
        assert_eq!(bpp2_palette_base(BgLayer::Bg3, 1), 0);
    }

    #[test]
    fn all_snes_bg_modes_have_render_mapping() {
        for mode in 0..=7 {
            for layer in [BgLayer::Bg1, BgLayer::Bg2, BgLayer::Bg3, BgLayer::Bg4] {
                assert!(BgRenderMode::from_bgmode(layer, mode).is_ok());
            }
        }
    }
}
