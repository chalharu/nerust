use nerust_snes_core::Core;

pub(super) fn read_tilemap_entry(
    core: &Core,
    tilemap_base: usize,
    tilemap_width_tiles: usize,
    tile_x: usize,
    tile_y: usize,
) -> u16 {
    let quadrant_columns = tilemap_width_tiles / 32;
    let quadrant = (tile_y / 32) * quadrant_columns + (tile_x / 32);
    let quadrant_base = tilemap_base.wrapping_add(quadrant.wrapping_mul(2048));
    let entry_offset = ((tile_y % 32) * 32 + (tile_x % 32)) * 2;
    u16::from_le_bytes([
        core.peek_vram(quadrant_base + entry_offset),
        core.peek_vram(quadrant_base + entry_offset + 1),
    ])
}

#[cfg(test)]
fn decode_2bpp_pixel(tile: &[u8], x: usize, y: usize) -> u8 {
    let row = y * 2;
    let shift = 7 - x;
    let plane0 = (tile[row] >> shift) & 0x01;
    let plane1 = (tile[row + 1] >> shift) & 0x01;
    plane0 | (plane1 << 1)
}

pub(super) fn bg_chr_2bpp_pixel(core: &Core, tile_addr: usize, x: usize, y: usize) -> u8 {
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

pub(super) fn chr_4bpp_pixel(core: &Core, tile_addr: usize, x: usize, y: usize) -> u8 {
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

pub(super) fn bg_chr_8bpp_pixel(core: &Core, tile_addr: usize, x: usize, y: usize) -> u8 {
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

#[cfg(test)]
mod tests {
    use super::{decode_2bpp_pixel, decode_4bpp_pixel, decode_8bpp_pixel};

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
}
