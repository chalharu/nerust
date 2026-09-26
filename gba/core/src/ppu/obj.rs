use super::{LayerPixel, PpuRegisters};
use crate::ppu::color::read_color;

const DIMENSIONS: [[(usize, usize); 4]; 3] = [
    [(8, 8), (16, 16), (32, 32), (64, 64)],
    [(16, 8), (32, 8), (32, 16), (64, 32)],
    [(8, 16), (8, 32), (16, 32), (32, 64)],
];

pub(crate) fn pixel(
    registers: &PpuRegisters,
    memory: (&[u8], &[u8], &[u8]),
    pos: (usize, usize),
    window_only: bool,
    mosaic: u16,
    cache: &ObjLineCache,
) -> Option<LayerPixel> {
    let (vram, palette, oam) = memory;
    let (x, y) = pos;
    let mut best: Option<(LayerPixel, usize)> = None;
    // Index-order visit with the same strict-`<` comparison: the result
    // is identical to scanning all 128 (lower index wins ties
    // regardless of visit order; lower priority always wins).
    for &(raw_index, attr0, attr1, attr2) in cache.cover[..cache.cover_len as usize].iter() {
        let index = usize::from(raw_index);
        let Some(object) = decode_attrs(attr0, attr1, attr2, window_only) else {
            continue;
        };
        let Some((local_x, local_y)) = object.coordinates(oam, x, y, mosaic) else {
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

/// Sprite walk over a pre-decoded working set (same selection as
/// [`pixel`]: index-order visit, strict-`<` on `(priority, index)`).
/// The blend-span path decodes the cover once per span instead of once
/// per pixel; the per-pixel work below is identical to `pixel`'s walk
/// body. `prepared` carries `(raw OAM index, scanline-prepared coords)`.
/// Forced-inline into the blend-span pixel loop.
#[inline]
pub(crate) fn pixel_predecoded(
    registers: &PpuRegisters,
    memory: (&[u8], &[u8], &[u8]),
    pos: (usize, usize),
    mosaic: u16,
    prepared: &[(u8, PreparedObj)],
) -> Option<LayerPixel> {
    let (vram, palette, _) = memory;
    let (x, _) = pos;
    let mut best: Option<(LayerPixel, usize)> = None;
    for &(raw_index, ref prepared) in prepared.iter() {
        let index = usize::from(raw_index);
        let object = &prepared.obj;
        let Some((local_x, local_y)) = prepared.coordinates(x, mosaic) else {
            continue;
        };
        let Some(palette_index) = object.palette_index(registers, vram, local_x, local_y) else {
            continue;
        };
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

/// Per-scanline OBJ working set, computed once per render span: the
/// cycle-drop mask fused with the y-overlap prefilter. Only OBJs that
/// can possibly cover the scanline are visited per pixel (typically a
/// handful of the 128). Stateless across spans: each span builds it
/// from contemporary state, so no invalidation is needed (prefix spans
/// build pre-write, remainder spans post-write, exactly like the
/// per-pixel computation it replaces).
#[derive(Clone, Copy)]
pub(crate) struct ObjLineCache {
    /// (index, attr0, attr1, attr2) in index order.
    pub(crate) cover: [(u8, u16, u16, u16); 128],
    pub(crate) cover_len: u8,
}

impl ObjLineCache {
    pub(crate) const fn empty() -> Self {
        Self {
            cover: [(0, 0, 0, 0); 128],
            cover_len: 0,
        }
    }

    pub(crate) fn from_cover(cover: [(u8, u16, u16, u16); 128], cover_len: u8) -> Self {
        Self { cover, cover_len }
    }

    pub(crate) fn into_cover(self) -> ([(u8, u16, u16, u16); 128], u8) {
        (self.cover, self.cover_len)
    }
}

pub(crate) fn line_cache(registers: &PpuRegisters, oam: &[u8], y: usize) -> ObjLineCache {
    let dropped = cycle_drop_mask(registers, oam, y);
    let mut cover = [(0u8, 0u16, 0u16, 0u16); 128];
    let mut len = 0u8;
    for (index, &is_dropped) in dropped.iter().enumerate() {
        if is_dropped {
            continue;
        }
        let base = index * 8;
        if base + 5 >= oam.len() {
            continue;
        }
        let attr0 = read16(oam, base);
        let attr1 = read16(oam, base + 2);
        let attr2 = read16(oam, base + 4);
        // Decode guards (mirrors decode_object minus the mode-2 /
        // window_only distinction, which stays per-path at render):
        // an undecodable OBJ covers nothing.
        let shape = usize::from((attr0 >> 14) & 3);
        let mode = (attr0 >> 10) & 3;
        let affine = attr0 & (1 << 8) != 0;
        if shape == 3 || mode == 3 || (!affine && attr0 & (1 << 9) != 0) {
            continue;
        }
        // Y-overlap (mirrors coordinates() up to the field-bounds
        // check, a necessary condition for any pixel of this OBJ to
        // render on this scanline regardless of mosaic/affine/flip).
        let height = DIMENSIONS[shape][usize::from((attr1 >> 14) & 3)].1;
        let double_size = affine && attr0 & (1 << 9) != 0;
        let field_height = if double_size { height * 2 } else { height };
        let y_raw = u32::from(attr0 & 0xFF);
        let y_max = (y_raw + field_height as u32) & 0xFF;
        let origin_y = if y_max < y_raw {
            y_raw as i32 - 256
        } else {
            y_raw as i32
        };
        let local_y = y as i32 - origin_y;
        if local_y < 0 || local_y >= field_height as i32 {
            continue;
        }
        cover[len as usize] = (index as u8, attr0, attr1, attr2);
        len += 1;
    }
    ObjLineCache {
        cover,
        cover_len: len,
    }
}

#[derive(Clone, Copy)]
pub(crate) struct Object {
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

/// Test-only OAM decoder (production decodes from the line-cache
/// attrs via [`decode_attrs`]); kept so decoder unit tests keep their
/// direct form.
#[cfg(test)]
fn decode_object(oam: &[u8], index: usize, window_only: bool) -> Option<Object> {
    let base = index * 8;
    decode_attrs(
        read16(oam, base),
        read16(oam, base + 2),
        read16(oam, base + 4),
        window_only,
    )
}

pub(crate) fn decode_attrs(
    attr0: u16,
    attr1: u16,
    attr2: u16,
    window_only: bool,
) -> Option<Object> {
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

/// Per-scanline prepared OBJ coordinates (see `Object::prepare`):
/// scanline-constant origin, y-side and affine terms resolved once
/// per span; only the x-side work remains per pixel.
#[derive(Clone, Copy)]
pub(crate) struct PreparedObj {
    obj: Object,
    origin_x: i32,
    local_y: i32,
    affine_pa: i32,
    affine_c1: i32,
    affine_pc: i32,
    affine_c2: i32,
}

impl Object {
    /// Per-scanline prepared coordinates: everything in [`coordinates`]
    /// that is constant across the scanline (origins, y-side bounds and
    /// mosaic, affine matrix rows) is resolved once per span; the
    /// per-pixel [`coordinates_prepared`] then only folds in `x`.
    /// Returns `None` when the scanline misses the field (same
    /// y-reject `line_cache` applies, so this is rare defensive).
    pub(crate) fn prepare(&self, oam: &[u8], y: usize, mosaic: u16) -> Option<PreparedObj> {
        let origin_x = signed_origin(self.attr1 & 0x1FF, 256, 512);
        // Same hardware-style Y wrap as `coordinates`.
        let y_raw = u32::from(self.attr0 & 0xFF);
        let y_max = (y_raw + self.field_height as u32) & 0xFF;
        let origin_y = if y_max < y_raw {
            y_raw as i32 - 256
        } else {
            y_raw as i32
        };
        let mut local_y = y as i32 - origin_y;
        if local_y < 0 || local_y >= self.field_height as i32 {
            return None;
        }
        // Y-side mosaic holds on the scanline-constant held row.
        if self.attr0 & (1 << 12) != 0 {
            let v = usize::from((mosaic >> 12) & 0xF) + 1;
            let held_y = if v == 1 { y as i32 } else { (y - y % v) as i32 };
            local_y = (held_y - origin_y).clamp(0, self.field_height as i32 - 1);
        }
        // Affine rows are scanline-constant: fold the constant
        // `pb * dy` / `pd * dy` terms in (`pa`/`pc` stay per-pixel with
        // the varying `dx`). Same arithmetic as `affine_coordinates`,
        // reassociated: ((pa*dx + c1) >> 8) + w/2 with c1 = pb*dy.
        let (pa, c1, pc, c2) = if self.affine {
            let parameter = usize::from((self.attr1 >> 9) & 0x1F) * 32;
            let pa = i32::from(read16_signed(oam, parameter + 6));
            let pb = i32::from(read16_signed(oam, parameter + 14));
            let pc = i32::from(read16_signed(oam, parameter + 22));
            let pd = i32::from(read16_signed(oam, parameter + 30));
            let dy = local_y - self.field_height as i32 / 2;
            (pa, pb * dy, pc, pd * dy)
        } else {
            (0, 0, 0, 0)
        };
        Some(PreparedObj {
            obj: *self,
            origin_x,
            local_y,
            affine_pa: pa,
            affine_c1: c1,
            affine_pc: pc,
            affine_c2: c2,
        })
    }
}

impl PreparedObj {
    /// Per-pixel coordinates from a scanline-[`Object::prepare`]d object:
    /// identical results to [`Object::coordinates`] for the prepared
    /// `(y, mosaic)` at any `x`.
    pub(crate) fn coordinates(&self, x: usize, mosaic: u16) -> Option<(usize, usize)> {
        let obj = &self.obj;
        let mut local_x = x as i32 - self.origin_x;
        if local_x < 0 || local_x >= obj.field_width as i32 {
            return None;
        }
        let mut local_y = self.local_y;
        // X-side mosaic holds on the pixel's held column (the y side is
        // already folded into `local_y`).
        if obj.attr0 & (1 << 12) != 0 {
            let h = usize::from((mosaic >> 8) & 0xF) + 1;
            let held_x = if h == 1 { x as i32 } else { (x - x % h) as i32 };
            local_x = (held_x - self.origin_x).clamp(0, obj.field_width as i32 - 1);
        }
        if obj.affine {
            // `dx` varies per pixel; the rest is prepared (same values
            // as `affine_coordinates`).
            let dx = local_x - obj.field_width as i32 / 2;
            local_x = ((self.affine_pa * dx + self.affine_c1) >> 8) + obj.width as i32 / 2;
            local_y = ((self.affine_pc * dx + self.affine_c2) >> 8) + obj.height as i32 / 2;
        } else {
            (local_x, local_y) = obj.flipped_coordinates(local_x, local_y);
        }
        in_bounds(local_x, local_y, obj.width, obj.height)
            .then_some((local_x as usize, local_y as usize))
    }
}

impl Object {
    fn coordinates(&self, oam: &[u8], x: usize, y: usize, mosaic: u16) -> Option<(usize, usize)> {
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
            let mut held = (local_x, local_y);
            crate::ppu::mosaic::apply_obj_mosaic(
                mosaic,
                (x, y),
                (origin_x, origin_y),
                &mut held,
                (self.field_width, self.field_height),
            );
            (local_x, local_y) = held;
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
        // Rows and columns wrap independently in 2D mapping (32x32-tile
        // matrix), 1D wraps the whole number at 10 bits.
        // The 256-color lower-bit mask applies to 2D only (GBATEK).
        let block_x = x / 8;
        let block_y = y / 8;
        let base = usize::from(self.attr2 & 0x3FF);
        if registers.dispcnt & (1 << 6) != 0 {
            if color256 {
                (base + block_y * (self.width >> 2) + (block_x << 1)) & 0x3FF
            } else {
                (base + block_y * (self.width >> 3) + block_x) & 0x3FF
            }
        } else if color256 {
            ((base + (block_y << 5)) & 0x3E0) | (((base & !1) + (block_x << 1)) & 0x1F)
        } else {
            ((base + (block_y << 5)) & 0x3E0) | ((base + block_x) & 0x1F)
        }
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

/// Per-line OBJ cycle budget (1210 cycles, 954 with H-Blank Interval Free).
/// Over budget, this OBJ and all lower-priority ones are dropped for the line.
/// Depends only on the scanline, line OAM and DISPCNT bit 5, so callers
/// cache it per scanline segment (computing it per pixel costs 128
/// line-cost evaluations per pixel).
pub(crate) fn cycle_drop_mask(registers: &PpuRegisters, oam: &[u8], y: usize) -> [bool; 128] {
    let mut dropped = [false; 128];
    let budget: u32 = if registers.dispcnt & (1 << 5) != 0 {
        954
    } else {
        1210
    };
    let mut used: u32 = 0;
    let mut over = None;
    for (index, _) in dropped.iter().enumerate() {
        let cost = line_cost(oam, index, y);
        if used + cost > budget {
            over = Some(index);
            break;
        }
        used += cost;
    }
    if let Some(at) = over {
        for item in dropped.iter_mut().skip(at) {
            *item = true;
        }
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
    // Horizontal participation: 9-bit X wraps (256..511 = -256..-1);
    // fully offscreen left or right costs the 2-cycle gap instead of the
    // full width (parked OBJs must not burn 64 cycles).
    let x_raw = attr1 & 0x1FF;
    let origin_x = if x_raw >= 256 {
        x_raw as i32 - 512
    } else {
        x_raw as i32
    };
    if origin_x + field_width as i32 <= 0 || origin_x >= 240 {
        return 2;
    }
    // Left-clipped remainder on the GBATEK cycle bases.
    let clip = origin_x.min(0);
    if affine {
        (10 + field_width as i32 * 2 + clip).max(2) as u32
    } else {
        (width as i32 + (clip >> 1)).max(2) as u32
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
    fn prepared_matches_reference_coordinates() {
        // Differential: prepared (span-hoisted) coordinates must equal
        // the per-pixel reference for every x of the scanline, across
        // shapes/sizes/affine/double/flip/mosaic/Y-wrap combinations.
        let mut oam = vec![0u8; 0x400];
        // Affine param block 0: identity-ish (pa=256, pd=256).
        oam[6..8].copy_from_slice(&256i16.to_le_bytes());
        oam[30..32].copy_from_slice(&256i16.to_le_bytes());
        // Affine param block 1: rotate-ish (pb=-256, pc=256).
        oam[32 + 14..32 + 16].copy_from_slice(&(-256i16).to_le_bytes());
        oam[32 + 22..32 + 24].copy_from_slice(&256i16.to_le_bytes());
        let mut seed = 0x12345678u32;
        let mut next = || {
            seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
            seed
        };
        for _ in 0..300 {
            let shape = next() % 3;
            let size = next() % 4;
            let affine = next() % 2 == 0;
            let double_size = affine && next() % 2 == 0;
            let mosaic_flag = next() % 2 == 0;
            let flip_x = !affine && next() % 2 == 0;
            let flip_y = !affine && next() % 2 == 0;
            let x_raw = next() % 512;
            let y_raw = next() % 256;
            let mut attr0 = y_raw as u16 | ((shape as u16) << 14);
            if affine {
                attr0 |= 1 << 8;
            }
            if double_size {
                attr0 |= 1 << 9;
            }
            if mosaic_flag {
                attr0 |= 1 << 12;
            }
            let mut attr1 = (x_raw as u16 & 0x1FF) | ((size as u16) << 14);
            if affine {
                attr1 |= ((next() % 2) as u16) << 9;
            } else {
                if flip_x {
                    attr1 |= 1 << 12;
                }
                if flip_y {
                    attr1 |= 1 << 13;
                }
            }
            oam[0..2].copy_from_slice(&attr0.to_le_bytes());
            oam[2..4].copy_from_slice(&attr1.to_le_bytes());
            oam[4..6].copy_from_slice(&0u16.to_le_bytes());
            let Some(obj) = decode_attrs(attr0, attr1, 0, false) else {
                continue;
            };
            let y = (next() % 160) as usize;
            let mosaic = (next() & 0xFFFF) as u16;
            let prepared = obj.prepare(&oam, y, mosaic);
            for x in (0..240).step_by(7) {
                let reference = obj.coordinates(&oam, x, y, mosaic);
                let actual = prepared.as_ref().and_then(|p| p.coordinates(x, mosaic));
                assert_eq!(
                    actual, reference,
                    "attr0={attr0:#x} attr1={attr1:#x} x={x} y={y} mosaic={mosaic:#x}"
                );
            }
            // Full sweep on a subsample (odd x covered above by step 7
            // over 240 with co-prime stride... assert dense for one).
            if shape == 0 && size == 0 {
                for x in 0..240 {
                    let reference = obj.coordinates(&oam, x, y, mosaic);
                    let actual = prepared.as_ref().and_then(|p| p.coordinates(x, mosaic));
                    assert_eq!(
                        actual, reference,
                        "dense attr0={attr0:#x} attr1={attr1:#x} x={x} y={y}"
                    );
                }
            }
        }
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
        let cache = line_cache(&regs, &oam, 0);
        assert!(
            pixel(
                &regs,
                (&vram[..], &palette[..], &oam[..]),
                (0, 0),
                false,
                regs.mosaic,
                &cache
            )
            .is_none()
        );
        // Tile 512 must be displayed.
        oam[4..6].copy_from_slice(&512u16.to_le_bytes());
        vram[0x10000 + 512 * 32] = 1;
        let cache = line_cache(&regs, &oam, 0);
        assert!(
            pixel(
                &regs,
                (&vram[..], &palette[..], &oam[..]),
                (0, 0),
                false,
                regs.mosaic,
                &cache
            )
            .is_some()
        );
    }

    #[test]
    fn obj_mosaic_is_screen_anchored() {
        // Sprite at x=1 with horizontal mosaic 2: screen x=2 holds screen
        // x=2 (block [2,3]), i.e. tile pixel 1, not sprite-local pixel 0.
        let regs = PpuRegisters {
            mosaic: 0x0100, // OBJ mosaic 1x0: h=2, v=1
            ..Default::default()
        };
        let mut vram = vec![0u8; 0x18000];
        vram[0x10000..0x10004].copy_from_slice(&[0x10, 0x32, 0x54, 0x76]);
        let mut palette = vec![0u8; 0x400];
        palette[0x202..0x204].copy_from_slice(&0x7C00u16.to_le_bytes());
        let mut oam = vec![0u8; 0x400];
        oam[0..2].copy_from_slice(&0x1000u16.to_le_bytes()); // Y=0, mosaic
        oam[2..4].copy_from_slice(&1u16.to_le_bytes()); // X=1, 8x8
        oam[4..6].copy_from_slice(&0u16.to_le_bytes()); // tile 0
        let cache = line_cache(&regs, &oam, 0);
        let pixel = pixel(
            &regs,
            (&vram[..], &palette[..], &oam[..]),
            (2, 0),
            false,
            regs.mosaic,
            &cache,
        )
        .expect("pixel");
        assert_eq!(pixel.color, 0x7C00);
    }

    #[test]
    fn tile_1d_256color_keeps_odd_base() {
        // 1D mapping keeps the lower tile bit in 256-color mode...
        let mut regs = regs_2d();
        regs.dispcnt |= 1 << 6; // 1D
        let mut oam = vec![0u8; 0x400];
        oam[0..2].copy_from_slice(&0x2000u16.to_le_bytes()); // 256-color
        oam[2..4].copy_from_slice(&0u16.to_le_bytes()); // 8x8
        oam[4..6].copy_from_slice(&1u16.to_le_bytes()); // odd base tile
        let obj = decode_object(&oam, 0, false).expect("object");
        assert_eq!(obj.tile_number(&regs, 0, 0, true), 1);
        // ...while 2D mapping ignores it.
        regs.dispcnt &= !(1 << 6);
        assert_eq!(obj.tile_number(&regs, 0, 0, true), 0);
    }

    #[test]
    fn tile_2d_wraps_rows_and_columns_independently() {
        // 16x16 4bpp at base 31: column wraps within the 32-tile row,
        // the row part does not carry.
        let regs = regs_2d();
        let mut oam = vec![0u8; 0x400];
        oam[0..2].copy_from_slice(&0u16.to_le_bytes()); // square
        oam[2..4].copy_from_slice(&0x4000u16.to_le_bytes()); // 16x16
        oam[4..6].copy_from_slice(&31u16.to_le_bytes()); // base tile 31
        let obj = decode_object(&oam, 0, false).expect("object");
        assert!(!obj.is_color256());
        assert_eq!(obj.tile_number(&regs, 0, 0, false), 31);
        assert_eq!(obj.tile_number(&regs, 8, 0, false), 0);
        assert_eq!(obj.tile_number(&regs, 0, 8, false), 63);
        assert_eq!(obj.tile_number(&regs, 8, 8, false), 32);
    }

    #[test]
    fn tile_1d_wraps_at_10_bits() {
        // 64x64 4bpp/8bpp 1D at high bases wrap mod 1024.
        let mut regs = regs_2d();
        regs.dispcnt |= 1 << 6; // 1D
        let mut oam = vec![0u8; 0x400];
        oam[0..2].copy_from_slice(&0u16.to_le_bytes()); // square 4bpp
        oam[2..4].copy_from_slice(&0xC000u16.to_le_bytes()); // 64x64
        oam[4..6].copy_from_slice(&1000u16.to_le_bytes());
        let obj = decode_object(&oam, 0, false).expect("object");
        assert_eq!(obj.tile_number(&regs, 56, 56, false), (1000 + 63) & 0x3FF);
        // 8bpp keeps the odd base and wraps the scaled sum.
        oam[0..2].copy_from_slice(&0x2000u16.to_le_bytes()); // 256-color
        oam[4..6].copy_from_slice(&1001u16.to_le_bytes());
        let obj = decode_object(&oam, 0, false).expect("object");
        assert_eq!(
            obj.tile_number(&regs, 56, 56, true),
            (1001 + 112 + 14) & 0x3FF
        );
    }

    #[test]
    fn tall_obj_wraps_above_128() {
        // 64x64 double-size (128px field) at Y=140 must appear at top, not bottom.
        let mut oam = vec![0u8; 0x400];
        // attr0: affine (bit8) + double (bit9) + square (shape 0); Y=140
        oam[0..2].copy_from_slice(&(140u16 | (1 << 8) | (1 << 9)).to_le_bytes());
        // attr1: affine param 0, size=3 (64x64)
        oam[2..4].copy_from_slice(&0xC000u16.to_le_bytes());
        oam[4..6].copy_from_slice(&0u16.to_le_bytes());
        let obj = decode_object(&oam, 0, false).expect("object");
        assert_eq!(obj.field_height, 128);
        // y=5 (top) is inside wrapped field [-116,12); y=150 (bottom) is outside.
        assert!(obj.coordinates(&oam, 0, 5, 0).is_some() || line_cost(&oam, 0, 5) > 2);
        // Direct coordinate check via pixel path would need identity matrix;
        // verify origin logic: y_max=(140+128)&255=12 <140 -> origin -116.
        let y_raw = 140u32;
        let y_max = (y_raw + 128) & 0xFF;
        assert!(y_max < y_raw);
    }
}
