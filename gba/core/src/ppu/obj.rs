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
    let mut best: Option<(LayerPixel, usize)> = None;
    for index in (0..128).rev() {
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
        let origin_y = signed_origin(self.attr0 & 0xFF, 160, 256);
        let mut local_x = x as i32 - origin_x;
        let mut local_y = y as i32 - origin_y;
        if !in_bounds(local_x, local_y, self.field_width, self.field_height) {
            return None;
        }
        if self.attr0 & (1 << 12) != 0 {
            apply_mosaic(registers.mosaic, &mut local_x, &mut local_y);
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
        let per_row = if registers.dispcnt & (1 << 6) != 0 {
            self.width / 8
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

fn in_bounds(x: i32, y: i32, width: usize, height: usize) -> bool {
    x >= 0 && y >= 0 && x < width as i32 && y < height as i32
}

fn apply_mosaic(mosaic: u16, x: &mut i32, y: &mut i32) {
    let horizontal = i32::from((mosaic >> 8) & 0xF) + 1;
    let vertical = i32::from((mosaic >> 12) & 0xF) + 1;
    *x -= *x % horizontal;
    *y -= *y % vertical;
}

fn read16(data: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes([data[offset], data[offset + 1]])
}

fn read16_signed(data: &[u8], offset: usize) -> i16 {
    i16::from_le_bytes([data[offset], data[offset + 1]])
}
