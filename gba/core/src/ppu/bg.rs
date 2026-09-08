use super::{LayerPixel, PpuRegisters};
use crate::ppu::color::read_color;
use crate::ppu::mosaic::bg_mosaic;

/// Per-pixel BG render context: registers, affine accumulators, memories,
/// and the hardware BG VRAM fetch latch (NBA `vram_bg_latch`).
struct BgContext<'a> {
    registers: &'a PpuRegisters,
    internal: ([i32; 2], [i32; 2]),
    vram: &'a [u8],
    palette: &'a [u8],
    latch: &'a mut u16,
    boundary: usize,
}

impl BgContext<'_> {
    /// BG VRAM fetch with the hardware fetch latch (NBA `FetchVRAM_BG`):
    /// reads below the OBJ boundary update the latch with the aligned
    /// halfword; reads at/above it return the latched bytes instead of
    /// physical VRAM.
    fn fetch_byte(&mut self, offset: usize) -> u8 {
        if offset >= self.boundary {
            self.latch_byte(offset)
        } else {
            *self.latch = read16(self.vram, offset & !1);
            self.vram[offset]
        }
    }

    fn fetch_half(&mut self, offset: usize) -> u16 {
        if offset >= self.boundary {
            *self.latch
        } else {
            *self.latch = read16(self.vram, offset);
            *self.latch
        }
    }

    fn latch_byte(&self, offset: usize) -> u8 {
        if offset & 1 == 0 {
            (*self.latch & 0xFF) as u8
        } else {
            (*self.latch >> 8) as u8
        }
    }
}

pub(crate) fn pixel(
    registers: &PpuRegisters,
    internal: ([i32; 2], [i32; 2]),
    memory: (&[u8], &[u8]),
    bg: usize,
    x: usize,
    y: usize,
    latch: &mut u16,
) -> Option<LayerPixel> {
    let (vram, palette) = memory;
    let mode = (registers.dispcnt & 7) as usize;
    let kind = match (mode, bg) {
        (0, _) | (1, 0 | 1) => BgKind::Text,
        (1, 2) | (2, 2 | 3) => BgKind::Affine,
        (3..=5, 2) => BgKind::Bitmap,
        _ => return None,
    };
    // OBJ VRAM boundary for BG fetches: 0x10000 in tile modes,
    // 0x14000 in bitmap modes (NBA `GetSpriteVRAMBoundary`).
    let boundary = if mode >= 3 { 0x14000 } else { 0x10000 };
    let cnt = registers.bgcnt[bg];
    let mut ctx = BgContext {
        registers,
        internal,
        vram,
        palette,
        latch,
        boundary,
    };
    let color = match kind {
        BgKind::Text => text_pixel(&mut ctx, bg, cnt, x, y)?,
        BgKind::Affine => affine_pixel(&mut ctx, bg, cnt, x, y)?,
        BgKind::Bitmap => bitmap_pixel(&mut ctx, mode, x, y)?,
    };
    Some(LayerPixel {
        color,
        priority: (cnt & 3) as u8,
        layer: bg as u8,
        semi_transparent: false,
    })
}

fn text_pixel(ctx: &mut BgContext<'_>, bg: usize, cnt: u16, x: usize, y: usize) -> Option<u16> {
    // Mosaic-held screen coords feed both the tile lookup and the sub-pixel
    // (output-latch equivalent: the whole block shows the origin pixel,
    // flip included). Scroll is added after, so blocks stay screen-fixed.
    let (mosaic_x, mosaic_y) = bg_mosaic(ctx.registers, cnt, x, y);
    let size = (cnt >> 14) & 3;
    let width = if size & 1 != 0 { 512 } else { 256 };
    let height = if size & 2 != 0 { 512 } else { 256 };
    let sx = (mosaic_x + usize::from(ctx.registers.hofs[bg])) & (width - 1);
    let sy = (mosaic_y + usize::from(ctx.registers.vofs[bg])) & (height - 1);
    let tile_x = sx / 8;
    let tile_y = sy / 8;
    let blocks_per_row = width / 256;
    let screen_block = tile_x / 32 + (tile_y / 32) * blocks_per_row;
    let map_base = usize::from((cnt >> 8) & 0x1F) * 0x800;
    let map_index = (tile_y % 32) * 32 + tile_x % 32;
    let entry_offset = map_base + screen_block * 0x800 + map_index * 2;
    let entry = ctx.fetch_half(entry_offset);
    let mut px = sx & 7;
    let mut py = sy & 7;
    if entry & (1 << 10) != 0 {
        px = 7 - px;
    }
    if entry & (1 << 11) != 0 {
        py = 7 - py;
    }
    let char_base = usize::from((cnt >> 2) & 3) * 0x4000;
    let tile = usize::from(entry & 0x3FF);
    if cnt & (1 << 7) != 0 {
        let offset = char_base + tile * 64 + py * 8 + px;
        let index = ctx.fetch_byte(offset);
        (index != 0).then(|| read_color(ctx.palette, usize::from(index)))
    } else {
        let offset = char_base + tile * 32 + py * 4 + px / 2;
        let packed = ctx.fetch_byte(offset);
        let index = if px & 1 == 0 {
            packed & 0xF
        } else {
            packed >> 4
        };
        let bank = usize::from((entry >> 12) & 0xF);
        (index != 0).then(|| read_color(ctx.palette, bank * 16 + usize::from(index)))
    }
}

fn affine_pixel(ctx: &mut BgContext<'_>, bg: usize, cnt: u16, x: usize, y: usize) -> Option<u16> {
    let affine = bg - 2;
    let (mx, my) = bg_mosaic(ctx.registers, cnt, x, y);
    let rel_x = mx as i32;
    // Vertical mosaic renders the held source line `my`: rewind the internal
    // reference (already advanced to line `y`) by the per-line increments.
    let mosaic_lines = y.saturating_sub(my) as i32;
    let line_x = ctx.internal.0[affine] - mosaic_lines * i32::from(ctx.registers.pb[affine]);
    let line_y = ctx.internal.1[affine] - mosaic_lines * i32::from(ctx.registers.pd[affine]);
    let mut sx = (line_x + rel_x * i32::from(ctx.registers.pa[affine])) >> 8;
    let mut sy = (line_y + rel_x * i32::from(ctx.registers.pc[affine])) >> 8;
    let size = 128i32 << ((cnt >> 14) & 3);
    if cnt & (1 << 13) != 0 {
        sx = sx.rem_euclid(size);
        sy = sy.rem_euclid(size);
    } else if sx < 0 || sy < 0 || sx >= size || sy >= size {
        return None;
    }
    let tiles_per_row = size as usize / 8;
    let map_base = usize::from((cnt >> 8) & 0x1F) * 0x800;
    let map_index = sy as usize / 8 * tiles_per_row + sx as usize / 8;
    let tile = usize::from(ctx.fetch_byte(map_base + map_index));
    let char_base = usize::from((cnt >> 2) & 3) * 0x4000;
    let offset = char_base + tile * 64 + (sy as usize & 7) * 8 + (sx as usize & 7);
    let index = ctx.fetch_byte(offset);
    (index != 0).then(|| read_color(ctx.palette, usize::from(index)))
}

fn bitmap_pixel(ctx: &mut BgContext<'_>, mode: usize, x: usize, y: usize) -> Option<u16> {
    let (mx, my) = bg_mosaic(ctx.registers, ctx.registers.bgcnt[2], x, y);
    let mosaic_lines = y.saturating_sub(my) as i32;
    let line_x = ctx.internal.0[0] - mosaic_lines * i32::from(ctx.registers.pb[0]);
    let line_y = ctx.internal.1[0] - mosaic_lines * i32::from(ctx.registers.pc[0]);
    let sx = (line_x + mx as i32 * i32::from(ctx.registers.pa[0])) >> 8;
    let sy = (line_y + mx as i32 * i32::from(ctx.registers.pc[0])) >> 8;
    let page = if ctx.registers.dispcnt & (1 << 4) != 0 {
        0xA000
    } else {
        0
    };
    match mode {
        3 if (0..240).contains(&sx) && (0..160).contains(&sy) => {
            Some(ctx.fetch_half((sy as usize * 240 + sx as usize) * 2))
        }
        4 if (0..240).contains(&sx) && (0..160).contains(&sy) => {
            // GBATEK Mode 4: color 0 is transparent (backdrop), so OBJs may
            // show behind the bitmap. Modes 3/5 are direct-color (opaque).
            let index = usize::from(ctx.fetch_byte(page + sy as usize * 240 + sx as usize));
            (index != 0).then(|| read_color(ctx.palette, index))
        }
        5 if (0..160).contains(&sx) && (0..128).contains(&sy) => {
            Some(ctx.fetch_half(page + (sy as usize * 160 + sx as usize) * 2))
        }
        _ => None,
    }
}

fn read16(data: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes([data[offset], data[offset + 1]])
}

enum BgKind {
    Text,
    Affine,
    Bitmap,
}
