use nerust_snes_core::Core;

use super::{
    BgLayer, RenderError, SCREEN_HEIGHT,
    color::cgram_raw_color,
    main_screen_for_line,
    mode7::render_mode7_bg1,
    presented_bg_line,
    tile::{bg_chr_2bpp_pixel, bg_chr_8bpp_pixel, chr_4bpp_pixel, read_tilemap_entry},
    use_presented_bg_scroll,
};
use nerust_snes_core::PresentedColorWindowLine;

pub(super) fn render_bg1(
    core: &Core,
    layer: BgLayer,
    brightness: u8,
    current_tm: u8,
    use_presented_tm: bool,
    interlace_enabled: bool,
    render_width: usize,
    render_height: usize,
    rgba: &mut [u8],
    raw_output: &mut [u16],
    hofs_extra: u16,
) -> Result<(), RenderError> {
    if !screen_uses_layer(core, layer, current_tm, use_presented_tm, render_height) {
        return Ok(());
    }

    let bgmode = core.peek(0x002105);
    let screen_mode = bgmode & 0x07;
    let Some(mode) = BgRenderMode::from_bgmode(layer, screen_mode)? else {
        return Ok(());
    };
    let high_res_mode = screen_mode == 5 || screen_mode == 6;
    // In Mode 5/6, tiles are always 16 pixels wide. The tile size bit
    // (BGMODE bits A/B/C/D) controls the height: 8 or 16 pixels tall.
    // In all other modes, tiles are square (8x8 or 16x16).
    let (tile_size, tile_height) = if high_res_mode {
        (16, if bgmode & layer.tile_size_mask() != 0 { 16 } else { 8 })
    } else {
        let s = if bgmode & layer.tile_size_mask() != 0 { 16 } else { 8 };
        (s, s)
    };
    if mode == BgRenderMode::Mode7 {
        render_mode7_bg1(
            core,
            brightness,
            current_tm,
            use_presented_tm,
            interlace_enabled,
            render_width,
            render_height,
            rgba,
        );
        return Ok(());
    }

    // For Mode7 we write directly to RGBA; for other modes we write to raw_output
    // No-op for raw_output in Mode7 path

    let bgsc = core.peek(layer.bgsc_register());
    let bg12nba = core.peek(0x00210B);
    let bg34nba = core.peek(0x00210C);
    // In Mode 5/6, tiles are always 16 pixels wide. The tile size bit
    // (BGMODE bits A/B/C/D) controls the height: 8 or 16 pixels tall.
    // In all other modes, tiles are square (8x8 or 16x16).
    let (tile_size, tile_height) = if high_res_mode {
        (16, if bgmode & layer.tile_size_mask() != 0 { 16 } else { 8 })
    } else {
        let s = if bgmode & layer.tile_size_mask() != 0 { 16 } else { 8 };
        (s, s)
    };
    let context = Bg1RenderContext {
        mode,
        tilemap_base: (usize::from(bgsc & 0xFC)) << 9,
        chr_base: layer.chr_base(bg12nba, bg34nba),
        tile_size,
        tile_height,
        tilemap_width_tiles: if bgsc & 0x01 != 0 { 64 } else { 32 },
        bpp2_palette_base: bpp2_palette_base(layer, screen_mode),
        high_res_mode,
    };
    let tilemap_height_tiles = if bgsc & 0x02 != 0 { 64 } else { 32 };
    let tilemap_width_pixels = context.tilemap_width_tiles * context.tile_size;
    let tilemap_height_pixels = tilemap_height_tiles * context.tile_height;
    let (current_hofs, current_vofs) = layer.current_scroll(core);
    let use_presented_scroll = use_presented_bg_scroll(core, layer);

    let hofs_mask = if high_res_mode { 0x7FF } else { 0x3FF };
    let height_ratio = (render_height / SCREEN_HEIGHT).max(1);

    // Window masking: TMW ($212E) controls which layers are masked
    // by the color window on the main screen.
    let tmw = core.peek(0x00212E);
    let layer_tm_mask = layer.tm_mask();
    let window_masked = tmw & layer_tm_mask != 0;
    let window_settings = if layer == BgLayer::Bg1 || layer == BgLayer::Bg2 {
        core.peek(0x002123)
    } else {
        core.peek(0x002124)
    };
    let layer_window_shift = match layer {
        BgLayer::Bg1 => 0,
        BgLayer::Bg2 => 4,
        BgLayer::Bg3 => 0,
        BgLayer::Bg4 => 4,
    };
    let win1_setting = (window_settings >> layer_window_shift) & 0x03;
    let win2_setting = (window_settings >> (layer_window_shift + 2)) & 0x03;
    let wbglog_shift = match layer {
        BgLayer::Bg1 => 0,
        BgLayer::Bg2 => 2,
        BgLayer::Bg3 => 4,
        BgLayer::Bg4 => 6,
    };
    let window_logic = (core.peek(0x00212A) >> wbglog_shift) & 0x03;

    // Mosaic effect: MOZA register ($2106)
    // Bits 0-3: mosaic size, Bits 4-7: enable for BG1-BG4
    let moza = core.peek(0x002106);
    let mosaic_size = usize::from(moza & 0x0F) + 1;
    let mosaic_enabled = (moza >> (4 + layer.bit_index())) & 0x01 != 0;

    for screen_y in 0..render_height {
        let presented_y = screen_y / height_ratio;
        if main_screen_for_line(core, presented_y, current_tm, use_presented_tm) & layer_tm_mask
            == 0
        {
            continue;
        }
        // Use per-scanline window data if available; otherwise fall back to
        // current register values (which retain the previous frame's HDMA writes).
        let window_line = core
            .presented_color_window_line(presented_y)
            .or_else(|| {
                // If no captured data, try the current frame's data from the "current" arrays
                // by calling presented_color_window_line with a different approach.
                None
            })
            .unwrap_or(PresentedColorWindowLine {
                wh0: core.peek(0x002126),
                wh1: core.peek(0x002127),
                wh2: core.peek(0x002128),
                wh3: core.peek(0x002129),
            });
        let wh0 = window_line.wh0;
        let wh1 = window_line.wh1;
        let wh2 = window_line.wh2;
        let wh3 = window_line.wh3;
        let presented = use_presented_scroll
            .then(|| presented_bg_line(core, layer, presented_y))
            .flatten();
        let hofs = (presented.map_or(usize::from(current_hofs.wrapping_add(hofs_extra)), |line| {
            usize::from(line.hofs.wrapping_add(hofs_extra))
        }) & hofs_mask)
            % tilemap_width_pixels.max(1);
        let raw_vofs = presented.map_or(current_vofs, |line| line.vofs);
        let interlace_field = interlace_enabled && (screen_y & 1) == 1;
        let effective_vofs = if interlace_field {
            (raw_vofs & 0x3FE) | 0x0001
        } else if interlace_enabled {
            raw_vofs & 0x3FE
        } else {
            raw_vofs & 0x3FF
        };
        let vofs = (usize::from(effective_vofs)) % tilemap_height_pixels.max(1);
        // The PPU's tile-fetch pipeline starts during the VBlank pre-render
        // scanline, loading the first tile row into an internal latch. Our stub
        // model captures each scanline's register state AFTER the latch has
        // been loaded (subtick 0 of each scanline). Consequently the bg_y
        // formula is: presented_y + vofs — no extra +1 needed.
        // The PPU fetches tiles for the first visible scanline during the
        // last VBlank scanline (pre-fetch). Our stub model does not simulate
        // this VBlank tile fetch, so we add 1 to presented_y to compensate:
        // the first visible scanline uses the tile data that would have been
        // pre-fetched at vofs + 0 during VBlank, making the effective offset
        // presented_y + 1 + vofs.
        // The PPU fetches tiles for the first visible scanline during the
        // last VBlank scanline (pre-fetch). Our stub model does not simulate
        // this VBlank tile fetch, so we add 1 to presented_y to compensate
        // for non-high-res modes. In high-resolution modes (5/6) tiles are
        // 16 pixels wide and the pre-fetch offset does not apply.
        let bg_y = if context.high_res_mode {
            (presented_y + vofs) % tilemap_height_pixels
        } else {
            (presented_y + 1 + vofs) % tilemap_height_pixels
        };
        // Mosaic: quantize Y to block boundary
        let mos_y = if mosaic_enabled {
            (screen_y / mosaic_size) * mosaic_size
        } else {
            screen_y
        };
        let pixel_bg_y = if mosaic_enabled {
            let extra = if context.high_res_mode { 0 } else { 1 };
            ((mos_y / height_ratio) + extra + vofs) % tilemap_height_pixels
        } else {
            bg_y
        };
        let row_offset = screen_y * render_width;
        for screen_x in 0..render_width {
            if window_masked
                && in_window(
                    win1_setting,
                    win2_setting,
                    window_logic,
                    screen_x,
                    wh0,
                    wh1,
                    wh2,
                    wh3,
                )
            {
                continue;
            }
            let bg_x = if mosaic_enabled {
                let mos_x = (screen_x / mosaic_size) * mosaic_size;
                (mos_x + hofs) % tilemap_width_pixels
            } else {
                (screen_x + hofs) % tilemap_width_pixels
            };
            if let Some(raw) = bg1_pixel(core, &context, bg_x, pixel_bg_y) {
                raw_output[row_offset + screen_x] = raw;
            }
        }
    }

    Ok(())
}

fn in_window(
    win1_setting: u8,
    win2_setting: u8,
    logic: u8,
    screen_x: usize,
    wh0: u8,
    wh1: u8,
    wh2: u8,
    wh3: u8,
) -> bool {
    // When WH0 > WH1 (inverted), the window covers ALL pixels.
    let inv1 = wh0 > wh1;
    let in_win1_range = if inv1 {
        true
    } else {
        (wh0..=wh1).contains(&(screen_x as u8))
    };
    let in_win1 = if win1_setting == 0 {
        false
    } else if win1_setting & 0x01 != 0 {
        // 01 = mask inside range; 11 = mask outside when non-inverted,
        // mask inside when inverted (inverted window covers all pixels,
        // so "mask inside" effectively masks everything).
        if win1_setting == 0x03 && !inv1 {
            !in_win1_range
        } else {
            in_win1_range
        }
    } else {
        !in_win1_range
    };

    let inv2 = wh2 > wh3;
    let in_win2_range = if inv2 {
        true
    } else {
        (wh2..=wh3).contains(&(screen_x as u8))
    };
    let in_win2 = if win2_setting == 0 {
        false
    } else if win2_setting & 0x01 != 0 {
        if win2_setting == 0x03 && !inv2 {
            !in_win2_range
        } else {
            in_win2_range
        }
    } else {
        !in_win2_range
    };

    match logic {
        0 => in_win1 || in_win2,
        1 => in_win1 && in_win2,
        2 => in_win1 ^ in_win2,
        _ => !(in_win1 ^ in_win2),
    }
}

fn screen_uses_layer(
    core: &Core,
    layer: BgLayer,
    current_tm: u8,
    use_presented_tm: bool,
    render_height: usize,
) -> bool {
    if !use_presented_tm {
        return current_tm & layer.tm_mask() != 0;
    }

    let height_ratio = (render_height / SCREEN_HEIGHT).max(1);
    (0..render_height).step_by(height_ratio).any(|screen_y| {
        main_screen_for_line(core, screen_y / height_ratio, current_tm, use_presented_tm)
            & layer.tm_mask()
            != 0
    })
}

fn bg1_pixel(core: &Core, context: &Bg1RenderContext, bg_x: usize, bg_y: usize) -> Option<u16> {
    let mut tile_x = bg_x / context.tile_size;
    let tile_y = bg_y / context.tile_height;
    let entry = read_tilemap_entry(
        core,
        context.tilemap_base,
        context.tilemap_width_tiles,
        tile_x,
        tile_y,
    );

    let mut tile_pixel_x = bg_x % context.tile_size;
    if context.high_res_mode {
        let opt = usize::from((entry >> 8) & 0x03);
        if opt > tile_pixel_x {
            tile_x = tile_x.wrapping_sub(1);
            let prev_entry = read_tilemap_entry(
                core,
                context.tilemap_base,
                context.tilemap_width_tiles,
                tile_x,
                tile_y,
            );
            return bg1_pixel_opt_wrapped(core, context, prev_entry, opt, tile_pixel_x, bg_y);
        }
        tile_pixel_x -= opt;
    }
    let mut tile_pixel_y = bg_y % context.tile_height;
    if entry & 0x4000 != 0 {
        tile_pixel_x = context.tile_size - 1 - tile_pixel_x;
    }
    if entry & 0x8000 != 0 {
        tile_pixel_y = context.tile_height - 1 - tile_pixel_y;
    }

    let subtile_x = tile_pixel_x / 8;
    let subtile_y = tile_pixel_y / 8;
    let pixel_x = tile_pixel_x % 8;
    let pixel_y = tile_pixel_y % 8;
    // VRAM tile stride: for standard modes (0-4) it is 16 tiles per row.
    // In high-resolution modes (5/6) the CHR data is stored contiguously
    // with a stride equal to the tile width in 8-pixel subtiles.
    let chr_row_stride = if context.high_res_mode {
        context.tile_size / 8
    } else {
        16
    };
    let tile_number = if context.high_res_mode {
        usize::from(entry & 0x00FF) + subtile_x + subtile_y * chr_row_stride
    } else {
        usize::from(entry & 0x03FF) + subtile_x + subtile_y * chr_row_stride
    };
    let tile_addr = context.chr_base + tile_number * context.mode.tile_bytes();
    let color = match context.mode {
        BgRenderMode::Bpp2 => bg_chr_2bpp_pixel(core, tile_addr, pixel_x, pixel_y),
        BgRenderMode::Bpp4 => chr_4bpp_pixel(core, tile_addr, pixel_x, pixel_y),
        BgRenderMode::Bpp8 => bg_chr_8bpp_pixel(core, tile_addr, pixel_x, pixel_y),
        BgRenderMode::Mode7 => unreachable!("Mode7 uses its own renderer"),
    };
    if color == 0 && context.mode != BgRenderMode::Bpp8 {
        return None;
    }

    let tile_palette = if context.high_res_mode {
        usize::from((entry >> 10) & 0x03)
    } else {
        usize::from((entry >> 10) & 0x07)
    };
    let color_index = match context.mode {
        BgRenderMode::Bpp2 => context.bpp2_palette_base + tile_palette * 4 + usize::from(color),
        BgRenderMode::Bpp4 => tile_palette * 16 + usize::from(color),
        BgRenderMode::Bpp8 => usize::from(color),
        BgRenderMode::Mode7 => unreachable!("Mode7 uses its own renderer"),
    };
    Some(cgram_raw_color(core, color_index))
}

fn bg1_pixel_opt_wrapped(
    core: &Core,
    context: &Bg1RenderContext,
    entry: u16,
    opt: usize,
    pixel_x_in: usize,
    bg_y: usize,
) -> Option<u16> {
    let mut tpix_x = pixel_x_in + context.tile_size - opt;
    let mut tile_pixel_y = bg_y % context.tile_height;
    if entry & 0x4000 != 0 {
        tpix_x = context.tile_size - 1 - tpix_x;
    }
    if entry & 0x8000 != 0 {
        tile_pixel_y = context.tile_height - 1 - tile_pixel_y;
    }
    let subtile_x = tpix_x / 8;
    let subtile_y = tile_pixel_y / 8;
    let pixel_x = tpix_x % 8;
    let pixel_y = tile_pixel_y % 8;
    let tile_number = usize::from(entry & 0x00FF) + subtile_x + subtile_y * 16;
    let tile_addr = context.chr_base + tile_number * context.mode.tile_bytes();
    let color = match context.mode {
        BgRenderMode::Bpp2 => bg_chr_2bpp_pixel(core, tile_addr, pixel_x, pixel_y),
        BgRenderMode::Bpp4 => chr_4bpp_pixel(core, tile_addr, pixel_x, pixel_y),
        BgRenderMode::Bpp8 => bg_chr_8bpp_pixel(core, tile_addr, pixel_x, pixel_y),
        BgRenderMode::Mode7 => unreachable!("Mode7 uses its own renderer"),
    };
    if color == 0 && context.mode != BgRenderMode::Bpp8 {
        return None;
    }
    let tile_palette = usize::from((entry >> 11) & 0x03);
    let color_index = match context.mode {
        BgRenderMode::Bpp2 => context.bpp2_palette_base + tile_palette * 4 + usize::from(color),
        BgRenderMode::Bpp4 => tile_palette * 16 + usize::from(color),
        BgRenderMode::Bpp8 => usize::from(color),
        BgRenderMode::Mode7 => unreachable!("Mode7 uses its own renderer"),
    };
    Some(cgram_raw_color(core, color_index))
}

#[derive(Debug, Clone, Copy)]
struct Bg1RenderContext {
    mode: BgRenderMode,
    tilemap_base: usize,
    chr_base: usize,
    tile_size: usize,
    tile_height: usize,
    tilemap_width_tiles: usize,
    bpp2_palette_base: usize,
    high_res_mode: bool,
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
            Self::Bg1 => 0x08,
            Self::Bg2 => 0x10,
            Self::Bg3 => 0x20,
            Self::Bg4 => 0x40,
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
