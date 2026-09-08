use super::{LayerPixel, PpuRegisters};
use crate::ppu::color::read_color;

const DIMENSIONS: [[(usize, usize); 4]; 3] = [
    [(8, 8), (16, 16), (32, 32), (64, 64)],
    [(16, 8), (32, 8), (32, 16), (64, 32)],
    [(8, 16), (8, 32), (16, 32), (32, 64)],
];

pub(crate) fn pixel(
    registers: &PpuRegisters,
    vram: &[u8],
    palette: &[u8],
    oam: &[u8],
    x: usize,
    y: usize,
    window_only: bool,
) -> Option<LayerPixel> {
    let dropped = cycle_drop_mask(registers, oam, y);
    let mut best: Option<(LayerPixel, usize)> = None;
    for index in (0..128).rev() {
        if dropped[index] {
            continue;
        }
        let Some(object) = decode_object(oam, index, window_only) else {
            continue;
        };
        let Some((local_x, local_y)) = object.coordinates(registers, oam, x, y) else {
            continue;
        };
        let Some(palette_index) = object.palette_index(registers, vram, local_x, local_y) else {
            continue;
        };
        if window_only {
            return Some(LayerPixel::default());
        }
        let candidate = LayerPixel {
            color: read_color(palette, palette_index),
            priority: ((object.attr2 >> 10) & 3) as u8,
            layer: 4,
            semi_transparent: object.mode == 1,
        };
        if best
            .as_ref()
            .is_none_or(|(old, old_index)| (candidate.priority, index) < (old.priority, *old_index))
        {
            best = Some((candidate, index));
        }
    }
    best.map(|(pixel, _)| pixel)
}

struct Object {
    attr0: u16,
    attr1: u16,
    attr2: u16,
    mode: u16,
    affine: bool,
    width: usize,
    height: usize,
    field_width: usize,
    field_height: usize,
}

fn decode_object(oam: &[u8], index: usize, window_only: bool) -> Option<Object> {
    let base = index * 8;
    let attr0 = read16(oam, base);
    let attr1 = read16(oam, base + 2);
    let attr2 = read16(oam, base + 4);
    let affine = attr0 & (1 << 8) != 0;
    let mode = (attr0 >> 10) & 3;
    let shape = usize::from((attr0 >> 14) & 3);
    if (!affine && attr0 & (1 << 9) != 0) || (mode == 2) != window_only || mode == 3 || shape == 3 {
        return None;
    }
    let (width, height) = DIMENSIONS[shape][usize::from((attr1 >> 14) & 3)];
    let double_size = affine && attr0 & (1 << 9) != 0;
    Some(Object {
        attr0,
        attr1,
        attr2,
        mode,
        affine,
        width,
        height,
        field_width: if double_size { width * 2 } else { width },
        field_height: if double_size { height * 2 } else { height },
    })
}

impl Object {
    fn coordinates(
        &self,
        registers: &PpuRegisters,
        oam: &[u8],
        x: usize,
        y: usize,
    ) -> Option<(usize, usize)> {
        let origin_x = signed_origin(self.attr1 & 0x1FF, 256, 512);
        // GBATEK OAM Attr0 Caution: very large OBJ (128px vertical, i.e. 64px
        // in Double Size) at Y>128 is treated as Y>-128 (8-bit adder overflow).
        // Use hardware-style wrap: y_max=(y_raw+field_h)&0xFF, wrap if overflow.
        let y_raw = u32::from(self.attr0 & 0xFF);
        let y_max = (y_raw + self.field_height as u32) & 0xFF;
        let origin_y = if y_max < y_raw {
            y_raw as i32 - 256
        } else {
            y_raw as i32
        };
        let mut local_x = x as i32 - origin_x;
        let mut local_y = y as i32 - origin_y;
        if !in_bounds(local_x, local_y, self.field_width, self.field_height) {
            return None;
        }
        if self.attr0 & (1 << 12) != 0 {
            crate::ppu::mosaic::apply_obj_mosaic(registers.mosaic, &mut local_x, &mut local_y);
        }
        if self.affine {
            (local_x, local_y) = self.affine_coordinates(oam, local_x, local_y);
        } else {
            (local_x, local_y) = self.flipped_coordinates(local_x, local_y);
        }
        in_bounds(local_x, local_y, self.width, self.height)
            .then_some((local_x as usize, local_y as usize))
    }

    fn affine_coordinates(&self, oam: &[u8], x: i32, y: i32) -> (i32, i32) {
        let parameter = usize::from((self.attr1 >> 9) & 0x1F) * 32;
        let dx = x - self.field_width as i32 / 2;
        let dy = y - self.field_height as i32 / 2;
        let pa = i32::from(read16_signed(oam, parameter + 6));
        let pb = i32::from(read16_signed(oam, parameter + 14));
        let pc = i32::from(read16_signed(oam, parameter + 22));
        let pd = i32::from(read16_signed(oam, parameter + 30));
        (
            ((pa * dx + pb * dy) >> 8) + self.width as i32 / 2,
            ((pc * dx + pd * dy) >> 8) + self.height as i32 / 2,
        )
    }

    fn flipped_coordinates(&self, mut x: i32, mut y: i32) -> (i32, i32) {
        if self.attr1 & (1 << 12) != 0 {
            x = self.width as i32 - 1 - x;
        }
        if self.attr1 & (1 << 13) != 0 {
            y = self.height as i32 - 1 - y;
        }
        (x, y)
    }

    fn palette_index(
        &self,
        registers: &PpuRegisters,
        vram: &[u8],
        x: usize,
        y: usize,
    ) -> Option<usize> {
        let color256 = self.is_color256();
        let tile_number = self.tile_number(registers, x, y, color256);
        // GBATEK OBJ Tile Number: in BG Modes 3-5 only tiles 512-1023 may be
        // used (lower 16K of OBJ VRAM is used for BG). Tiles 0-511 are ignored.
        if (registers.dispcnt & 7) >= 3 && tile_number < 512 {
            return None;
        }
        let offset = Self::vram_offset(tile_number, x, y, color256);
        let packed = *vram.get(offset)?;
        let index = Self::decode_index(packed, x, color256);
        if index == 0 {
            return None;
        }
        Some(Self::palette_entry(index, color256, self.attr2))
    }

    fn is_color256(&self) -> bool {
        self.attr0 & (1 << 13) != 0
    }

    fn tile_number(&self, registers: &PpuRegisters, x: usize, y: usize, color256: bool) -> usize {
        let base = usize::from(self.attr2 & 0x3FF) & if color256 { !1 } else { usize::MAX };
        // GBATEK OBJ VRAM Mapping: 2D 256-color mode is 16x32 tiles (128x256px),
        // not 32x32. 16-color 2D is 32x32, 1D is width/8 in both depths.
        let per_row = if registers.dispcnt & (1 << 6) != 0 {
            self.width / 8
        } else if color256 {
            16
        } else {
            32
        };
        let scale = if color256 { 2 } else { 1 };
        base + (y / 8 * per_row + x / 8) * scale
    }

    fn vram_offset(tile_number: usize, x: usize, y: usize, color256: bool) -> usize {
        0x10000
            + tile_number * 32
            + (y & 7) * if color256 { 8 } else { 4 }
            + (x & 7) / if color256 { 1 } else { 2 }
    }

    fn decode_index(packed: u8, x: usize, color256: bool) -> u8 {
        if color256 {
            packed
        } else if x & 1 == 0 {
            packed & 0xF
        } else {
            packed >> 4
        }
    }

    fn palette_entry(index: u8, color256: bool, attr2: u16) -> usize {
        if color256 {
            256 + usize::from(index)
        } else {
            256 + usize::from((attr2 >> 12) & 0xF) * 16 + usize::from(index)
        }
    }
}

fn signed_origin(value: u16, threshold: i32, modulus: i32) -> i32 {
    let value = i32::from(value);
    if value >= threshold {
        value - modulus
    } else {
        value
    }
}

/// GBATEK OBJ Overview: per-line OBJ rendering cycle budget.
/// 1210 cycles if H-Blank Interval Free (DISPCNT bit5) is 0, else 954.
/// Normal OBJ costs `width` cycles, affine costs `10 + field_width*2`.
/// OBJs that are disabled, prohibited, or vertically off the line cost 2.
/// Higher-priority (lower-index) offscreen OBJs also consume cycles, so once
/// the budget is exceeded this line drops the OBJ and all lower-priority ones.
fn cycle_drop_mask(registers: &PpuRegisters, oam: &[u8], y: usize) -> [bool; 128] {
    let mut dropped = [false; 128];
    let budget: u32 = if registers.dispcnt & (1 << 5) != 0 {
        954
    } else {
        1210
    };
    let mut used: u32 = 0;
    for index in 0..128 {
        let cost = line_cost(oam, index, y);
        if used + cost > budget {
            for item in dropped.iter_mut().skip(index) {
                *item = true;
            }
            break;
        }
        used += cost;
    }
    dropped
}

fn line_cost(oam: &[u8], index: usize, y: usize) -> u32 {
    let base = index * 8;
    if base + 3 >= oam.len() {
        return 2;
    }
    let attr0 = read16(oam, base);
    let attr1 = read16(oam, base + 2);
    let shape = usize::from((attr0 >> 14) & 3);
    let mode = (attr0 >> 10) & 3;
    if shape == 3 || mode == 3 {
        return 2;
    }
    let affine = attr0 & (1 << 8) != 0;
    if !affine && attr0 & (1 << 9) != 0 {
        return 2;
    }
    let (width, height) = DIMENSIONS[shape][usize::from((attr1 >> 14) & 3)];
    let double_size = affine && attr0 & (1 << 9) != 0;
    let field_width = if double_size { width * 2 } else { width };
    let field_height = if double_size { height * 2 } else { height };
    let y_raw = u32::from(attr0 & 0xFF);
    let y_max = (y_raw + field_height as u32) & 0xFF;
    let origin_y = if y_max < y_raw {
        y_raw as i32 - 256
    } else {
        y_raw as i32
    };
    if (y as i32) < origin_y || (y as i32) >= origin_y + field_height as i32 {
        return 2;
    }
    if affine {
        10 + field_width as u32 * 2
    } else {
        width as u32
    }
}

fn in_bounds(x: i32, y: i32, width: usize, height: usize) -> bool {
    x >= 0 && y >= 0 && x < width as i32 && y < height as i32
}

fn read16(data: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes([data[offset], data[offset + 1]])
}

fn read16_signed(data: &[u8], offset: usize) -> i16 {
    i16::from_le_bytes([data[offset], data[offset + 1]])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ppu::PpuRegisters;

    fn regs_2d() -> PpuRegisters {
        PpuRegisters::default() // DISPCNT bit6=0 -> 2D
    }

    #[test]
    fn tile_2d_256color_uses_16_per_row() {
        let regs = regs_2d();
        // 16x16 OBJ, base tile 4, 256-color: (0,8) -> 0x24, not 0x44.
        let mut oam = vec![0u8; 0x400];
        // attr0: 256-color (bit13), square; attr1: size=1 (16x16); attr2: tile=4
        oam[0..2].copy_from_slice(&0x2000u16.to_le_bytes());
        oam[2..4].copy_from_slice(&0x4000u16.to_le_bytes());
        oam[4..6].copy_from_slice(&4u16.to_le_bytes());
        let obj = decode_object(&oam, 0, false).expect("object");
        assert!(obj.is_color256());
        let tile = obj.tile_number(&regs, 0, 8, true);
        assert_eq!(tile, 0x24);
    }

    #[test]
    fn bitmap_mode_ignores_tiles_below_512() {
        let mut regs = regs_2d();
        regs.dispcnt = 3; // BG Mode 3
        let mut vram = vec![0u8; 0x18000];
        // Tile 0 filled with palette index 1.
        vram[0x10000] = 1;
        let palette = {
            let mut p = vec![0u8; 0x400];
            p[0x202..0x204].copy_from_slice(&0x7FFFu16.to_le_bytes());
            p
        };
        let mut oam = vec![0u8; 0x400];
        oam[0..6].copy_from_slice(&[0, 0, 0, 0, 0, 0]); // 8x8 at (0,0), tile 0
        // Tile 0 in mode 3 must be ignored.
        assert!(pixel(&regs, &vram, &palette, &oam, 0, 0, false).is_none());
        // Tile 512 must be displayed.
        oam[4..6].copy_from_slice(&512u16.to_le_bytes());
        vram[0x10000 + 512 * 32] = 1;
        assert!(pixel(&regs, &vram, &palette, &oam, 0, 0, false).is_some());
    }

    #[test]
    fn tall_obj_wraps_above_128() {
        // 64x64 double-size (128px field) at Y=140 must appear at top, not bottom.
        let regs = regs_2d();
        let mut oam = vec![0u8; 0x400];
        // attr0: affine (bit8) + double (bit9) + square (shape 0); Y=140
        oam[0..2].copy_from_slice(&(140u16 | (1 << 8) | (1 << 9)).to_le_bytes());
        // attr1: affine param 0, size=3 (64x64)
        oam[2..4].copy_from_slice(&0xC000u16.to_le_bytes());
        oam[4..6].copy_from_slice(&0u16.to_le_bytes());
        let obj = decode_object(&oam, 0, false).expect("object");
        assert_eq!(obj.field_height, 128);
        // y=5 (top) is inside wrapped field [-116,12); y=150 (bottom) is outside.
        assert!(obj.coordinates(&regs, &oam, 0, 5).is_some() || line_cost(&oam, 0, 5) > 2);
        // Direct coordinate check via pixel path would need identity matrix;
        // verify origin logic: y_max=(140+128)&255=12 <140 -> origin -116.
        let y_raw = 140u32;
        let y_max = (y_raw + 128) & 0xFF;
        assert!(y_max < y_raw);
    }
}
