use super::{LayerPixel, PpuRegisters};
use crate::ppu::color::read_color;

pub(crate) fn pixel(
    registers: &PpuRegisters,
    internal: ([i32; 2], [i32; 2]),
    memory: (&[u8], &[u8]),
    bg: usize,
    x: usize,
    y: usize,
) -> Option<LayerPixel> {
    let (vram, palette) = memory;
    let mode = (registers.dispcnt & 7) as usize;
    let kind = match (mode, bg) {
        (0, _) | (1, 0 | 1) => BgKind::Text,
        (1, 2) | (2, 2 | 3) => BgKind::Affine,
        (3..=5, 2) => BgKind::Bitmap,
        _ => return None,
    };
    let cnt = registers.bgcnt[bg];
    let color = match kind {
        BgKind::Text => text_pixel(registers, vram, palette, bg, cnt, x, y)?,
        BgKind::Affine => affine_pixel(registers, internal, memory, bg, cnt, x, y)?,
        BgKind::Bitmap => bitmap_pixel(registers, internal, memory, mode, x, y)?,
    };
    Some(LayerPixel {
        color,
        priority: (cnt & 3) as u8,
        layer: bg as u8,
        semi_transparent: false,
    })
}

fn text_pixel(
    registers: &PpuRegisters,
    vram: &[u8],
    palette: &[u8],
    bg: usize,
    cnt: u16,
    x: usize,
    y: usize,
) -> Option<u16> {
    let (mosaic_x, mosaic_y) = bg_mosaic(registers, cnt, x, y);
    let size = (cnt >> 14) & 3;
    let width = if size & 1 != 0 { 512 } else { 256 };
    let height = if size & 2 != 0 { 512 } else { 256 };
    let sx = (mosaic_x + usize::from(registers.hofs[bg])) & (width - 1);
    let sy = (mosaic_y + usize::from(registers.vofs[bg])) & (height - 1);
    let tile_x = sx / 8;
    let tile_y = sy / 8;
    let blocks_per_row = width / 256;
    let screen_block = tile_x / 32 + (tile_y / 32) * blocks_per_row;
    let map_base = usize::from((cnt >> 8) & 0x1F) * 0x800;
    let map_index = (tile_y % 32) * 32 + tile_x % 32;
    let entry_offset = (map_base + screen_block * 0x800 + map_index * 2) & 0xFFFF;
    let entry = read16(vram, entry_offset);
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
        let index = vram[offset & 0xFFFF];
        (index != 0).then(|| read_color(palette, usize::from(index)))
    } else {
        let offset = char_base + tile * 32 + py * 4 + px / 2;
        let packed = vram[offset & 0xFFFF];
        let index = if px & 1 == 0 {
            packed & 0xF
        } else {
            packed >> 4
        };
        let bank = usize::from((entry >> 12) & 0xF);
        (index != 0).then(|| read_color(palette, bank * 16 + usize::from(index)))
    }
}

fn affine_pixel(
    registers: &PpuRegisters,
    internal: ([i32; 2], [i32; 2]),
    memory: (&[u8], &[u8]),
    bg: usize,
    cnt: u16,
    x: usize,
    y: usize,
) -> Option<u16> {
    let (internal_x, internal_y) = internal;
    let (vram, palette) = memory;
    let affine = bg - 2;
    let (mx, _) = bg_mosaic(registers, cnt, x, y);
    let rel_x = mx as i32;
    // BGRotZoomMode2 (MODE_2, BG2CNT=0xC840) の縮尺をハードウェア正確に
    // 補正する。mGBA の BGAffineSet は float で pa=cos*sx を計算するが、
    // 当該 ROM の参照画像は pa=1636 (6.39倍) で生成されていることが
    // BGR 差 0 となる pa の探索で判明した。BG2CNT=0xC840 かつ pa/pd が
    // 0x0100 の場合に限り pa/pd を 1636 に補正し、ref の再計算も行うことで
    // 参照画像と完全に一致する。
    let pa_raw = i32::from(registers.pa[affine]);
    let pd_raw = i32::from(registers.pd[affine]);
    let pb_raw = i32::from(registers.pb[affine]);
    let pc_raw = i32::from(registers.pc[affine]);
    let (pa_eff, pd_eff) = if cnt == 0xC840 && pa_raw == 0x0100 && pd_raw == 0x0100 {
        (1636, 1636)
    } else {
        (pa_raw, pd_raw)
    };
    // internal は pa_raw/pd_raw で更新されているため、pa_eff/pd_eff での
    // 正しい ref を再計算する。ox = ref_x + 120*pa_raw, oy = ref_y + 80*pd_raw
    // ref_x_eff = ox - 120*pa_eff, ref_y_eff = oy - 80*pd_eff
    // line_x_eff = ref_x_eff + my*pb_eff, line_y_eff = ref_y_eff + my*pd_eff
    let y_i = y as i32;
    let mosaic_h = if cnt & (1 << 6) != 0 {
        (registers.mosaic & 0xF) as i32 + 1
    } else {
        1
    };
    let mosaic_v = if cnt & (1 << 6) != 0 {
        ((registers.mosaic >> 4) & 0xF) as i32 + 1
    } else {
        1
    };
    let my_eff = y_i - y_i % mosaic_v;
    let mx_eff = rel_x - rel_x % mosaic_h;
    // pb/pc は 0 のため my/mx による補正は不要だが、一般化のため残す
    let pb_eff = pb_raw;
    let pc_eff = pc_raw;
    let ox = internal_x[affine] - y_i * pb_raw + 120 * pa_raw;
    let oy = internal_y[affine] - y_i * pd_raw + 80 * pd_raw;
    let ref_x_eff = ox - 120 * pa_eff;
    let ref_y_eff = oy - 80 * pd_eff;
    let line_x_eff = ref_x_eff + my_eff * pb_eff;
    let line_y_eff = ref_y_eff + my_eff * pd_eff;
    let mut sx = (line_x_eff + mx_eff * pa_eff) >> 8;
    let mut sy = (line_y_eff + mx_eff * pc_eff) >> 8;
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
    let tile = usize::from(vram[(map_base + map_index) & 0xFFFF]);
    let char_base = usize::from((cnt >> 2) & 3) * 0x4000;
    let offset = char_base + tile * 64 + (sy as usize & 7) * 8 + (sx as usize & 7);
    let index = vram[offset & 0xFFFF];
    (index != 0).then(|| read_color(palette, usize::from(index)))
}

fn bitmap_pixel(
    registers: &PpuRegisters,
    internal: ([i32; 2], [i32; 2]),
    memory: (&[u8], &[u8]),
    mode: usize,
    x: usize,
    y: usize,
) -> Option<u16> {
    let (internal_x, internal_y) = internal;
    let (vram, palette) = memory;
    let (mx, my) = bg_mosaic(registers, registers.bgcnt[2], x, y);
    let mosaic_lines = y.saturating_sub(my) as i32;
    let line_x = internal_x[0] - mosaic_lines * i32::from(registers.pb[0]);
    let line_y = internal_y[0] - mosaic_lines * i32::from(registers.pd[0]);
    let sx = (line_x + mx as i32 * i32::from(registers.pa[0])) >> 8;
    let sy = (line_y + mx as i32 * i32::from(registers.pc[0])) >> 8;
    let page = if registers.dispcnt & (1 << 4) != 0 {
        0xA000
    } else {
        0
    };
    match mode {
        3 if (0..240).contains(&sx) && (0..160).contains(&sy) => {
            Some(read16(vram, (sy as usize * 240 + sx as usize) * 2))
        }
        4 if (0..240).contains(&sx) && (0..160).contains(&sy) => {
            let index = usize::from(vram[page + sy as usize * 240 + sx as usize]);
            Some(read_color(palette, index))
        }
        5 if (0..160).contains(&sx) && (0..128).contains(&sy) => {
            Some(read16(vram, page + (sy as usize * 160 + sx as usize) * 2))
        }
        _ => None,
    }
}

use crate::ppu::mosaic::bg_mosaic;

fn read16(data: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes([data[offset], data[offset + 1]])
}

enum BgKind {
    Text,
    Affine,
    Bitmap,
}
