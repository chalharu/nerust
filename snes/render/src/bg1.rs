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
    let context = Bg1RenderContext {
        mode,
        tilemap_base: (usize::from(bgsc & 0xFC)) << 9,
        chr_base: layer.chr_base(bg12nba, bg34nba),
        tile_size: if bgmode & layer.tile_size_mask() != 0 {
            16
        } else {
            8
        },
        tilemap_width_tiles: if bgsc & 0x01 != 0 { 64 } else { 32 },
        bpp2_palette_base: bpp2_palette_base(layer, screen_mode),
        high_res_mode,
    };
    let tilemap_height_tiles = if bgsc & 0x02 != 0 { 64 } else { 32 };
    let tilemap_width_pixels = context.tilemap_width_tiles * context.tile_size;
    let tilemap_height_pixels = tilemap_height_tiles * context.tile_size;
    let (current_hofs, current_vofs) = layer.current_scroll(core);
    let use_presented_scroll = use_presented_bg_scroll(core, layer);

    let hofs_mask = if high_res_mode { 0x7FF } else { 0x3FF };
    let height_ratio = (render_height / SCREEN_HEIGHT).max(1);

    for screen_y in 0..render_height {
        let presented_y = screen_y / height_ratio;
        if main_screen_for_line(core, presented_y, current_tm, use_presented_tm) & layer.tm_mask()
            == 0
        {
            continue;
        }
        let presented = use_presented_scroll
            .then(|| presented_bg_line(core, layer, presented_y))
            .flatten();
        let hofs = (presented.map_or(usize::from(current_hofs), |line| usize::from(line.hofs))
            & hofs_mask)
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
        let vofs =
            (usize::from(effective_vofs)) % tilemap_height_pixels.max(1);
        let bg_y = (presented_y + 1 + vofs) % tilemap_height_pixels;
        let row_offset = screen_y * render_width;
        for screen_x in 0..render_width {
            let bg_x = (screen_x + hofs) % tilemap_width_pixels;
            if let Some(raw) = bg1_pixel(core, &context, bg_x, bg_y) {
                raw_output[row_offset + screen_x] = raw;
            }
        }
    }

    Ok(())
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
    let tile_y = bg_y / context.tile_size;
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
    let tile_number = if context.high_res_mode {
        usize::from(entry & 0x00FF) + subtile_x + subtile_y * 16
    } else {
        usize::from(entry & 0x03FF) + subtile_x + subtile_y * 16
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
        usize::from((entry >> 11) & 0x03)
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
    let mut tile_pixel_y = bg_y % context.tile_size;
    if entry & 0x4000 != 0 {
        tpix_x = context.tile_size - 1 - tpix_x;
    }
    if entry & 0x8000 != 0 {
        tile_pixel_y = context.tile_size - 1 - tile_pixel_y;
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
