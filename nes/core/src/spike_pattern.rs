//! SPIKE-ONLY pattern-table producer (spike/debugger-ui-prototype-3).
//! Deleted with the spike branch. Validates the kernel `DebugImage`
//! shape, the CHR/palette read paths, and the palette decision.
//!
//! Palette decision under test: tiles decode to 2-bit indices; the
//! 4-entry palette comes from PPU palette RAM $3F00-$3F03 resolved
//! through the canonical 2C02 master table embedded below. Emphasis
//! bits, color-bit masking ($3F00 & $30), and filter consistency are
//! out of scope and recorded as open questions.

use nerust_core_traits::debugger::{DebugImage, ImageFormat};
use nerust_input_traits::OpenBusReadResult;

use crate::Core;

/// Canonical 2C02 master palette (standard 64-entry RGB table).
/// Hardware data, not per-ROM: any canonical variant validates the
/// descriptor shape. Emphasis handling is a production decision.
const MASTER_PALETTE: [[u8; 3]; 64] = [
    [0x7C, 0x7C, 0x7C],
    [0x00, 0x00, 0xFC],
    [0x00, 0x00, 0xBC],
    [0x44, 0x28, 0xBC],
    [0x94, 0x00, 0x84],
    [0xA8, 0x00, 0x20],
    [0xA8, 0x10, 0x00],
    [0x88, 0x14, 0x00],
    [0x50, 0x30, 0x00],
    [0x00, 0x78, 0x00],
    [0x00, 0x68, 0x00],
    [0x00, 0x58, 0x00],
    [0x00, 0x40, 0x58],
    [0x00, 0x00, 0x00],
    [0x00, 0x00, 0x00],
    [0x00, 0x00, 0x00],
    [0xBC, 0xBC, 0xBC],
    [0x00, 0x78, 0xF8],
    [0x00, 0x58, 0xF8],
    [0x68, 0x44, 0xFC],
    [0xD8, 0x28, 0xB8],
    [0xE4, 0x00, 0x58],
    [0xF8, 0x38, 0x00],
    [0xE4, 0x5C, 0x10],
    [0xAC, 0x7C, 0x00],
    [0x00, 0xB8, 0x00],
    [0x00, 0xA8, 0x00],
    [0x00, 0xA8, 0x44],
    [0x00, 0x88, 0x88],
    [0x00, 0x00, 0x00],
    [0x00, 0x00, 0x00],
    [0x00, 0x00, 0x00],
    [0xF8, 0xF8, 0xF8],
    [0x3C, 0xBC, 0xFC],
    [0x68, 0x88, 0xFC],
    [0x98, 0x78, 0xF8],
    [0xF8, 0x78, 0xF8],
    [0xF8, 0x58, 0x98],
    [0xF8, 0x78, 0x58],
    [0xFC, 0xA0, 0x44],
    [0xF8, 0xB8, 0x00],
    [0xB8, 0xF8, 0x18],
    [0x58, 0xD8, 0x54],
    [0x58, 0xF8, 0x98],
    [0x00, 0xE8, 0xD8],
    [0x78, 0x78, 0x78],
    [0x00, 0x00, 0x00],
    [0x00, 0x00, 0x00],
    [0xFC, 0xFC, 0xFC],
    [0xA4, 0xE4, 0xFC],
    [0xB8, 0xB8, 0xF8],
    [0xD8, 0xB8, 0xF8],
    [0xF8, 0xB8, 0xF8],
    [0xF8, 0xA8, 0xC0],
    [0xF0, 0xD0, 0xB0],
    [0xFC, 0xE0, 0xA8],
    [0xF8, 0xD8, 0x78],
    [0xD8, 0xF8, 0x78],
    [0xB8, 0xF8, 0xB8],
    [0xB8, 0xF8, 0xD8],
    [0x00, 0xFC, 0xFC],
    [0xF8, 0xD8, 0xF8],
    [0x00, 0x00, 0x00],
    [0x00, 0x00, 0x00],
];

/// Decode one 8x8 tile (16 CHR bytes) to 64 2-bit indices, row-major.
pub fn decode_tile(bytes: &[u8; 16]) -> [u8; 64] {
    let mut out = [0u8; 64];
    for row in 0..8 {
        let lo = bytes[row];
        let hi = bytes[row + 8];
        for col in 0..8 {
            let bit = 7 - col;
            out[row * 8 + col] = ((lo >> bit) & 1) | (((hi >> bit) & 1) << 1);
        }
    }
    out
}

fn chr_byte(core: &Core, addr: usize) -> Option<u8> {
    core.peek_chr_byte(addr)
        .and_then(|read: OpenBusReadResult| (read.mask == 0xFF).then_some(read.data))
}

fn palette_entry(core: &Core, addr: usize) -> [u8; 3] {
    // Palette RAM via the VRAM peek (mirrors resolved inside).
    // Floating reads fall back to entry 0 (universal background).
    let index = core
        .peek_ppu_vram(addr)
        .map(|v| (v & 0x3F) as usize)
        .unwrap_or(0);
    MASTER_PALETTE[index]
}

/// Build the two pattern-table images (left $0000, right $1000).
/// Each is 128x128 (16x16 tiles). Palette is BG palette 0
/// ($3F00-$3F03). Tiles with any unreadable byte decode as blank.
pub fn pattern_images(core: &Core) -> Vec<DebugImage> {
    let palette: Vec<[u8; 3]> = (0..4).map(|i| palette_entry(core, 0x3F00 + i)).collect();
    [("pattern-left", 0x0000), ("pattern-right", 0x1000)]
        .into_iter()
        .map(|(id, base)| {
            let mut pixels = vec![0u8; 128 * 128];
            for tile in 0..256 {
                let mut bytes = [0u8; 16];
                let mut ok = true;
                for (i, slot) in bytes.iter_mut().enumerate() {
                    match chr_byte(core, base + tile * 16 + i) {
                        Some(b) => *slot = b,
                        None => {
                            ok = false;
                            break;
                        }
                    }
                }
                if !ok {
                    continue;
                }
                let tile_px = decode_tile(&bytes);
                let tx = (tile % 16) * 8;
                let ty = (tile / 16) * 8;
                for row in 0..8 {
                    for col in 0..8 {
                        pixels[(ty + row) * 128 + tx + col] = tile_px[row * 8 + col];
                    }
                }
            }
            DebugImage {
                id,
                label_id: if id == "pattern-left" {
                    "Pattern left"
                } else {
                    "Pattern right"
                },
                width: 128,
                height: 128,
                format: ImageFormat::Indexed2bpp,
                palette: palette.clone(),
                pixels,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decode_tile_planes_combine() {
        // Row 0: lo=0b10000001, hi=0b01000010 -> indices 1,2,0..0,2,1.
        let mut bytes = [0u8; 16];
        bytes[0] = 0b1000_0001;
        bytes[8] = 0b0100_0010;
        let px = decode_tile(&bytes);
        assert_eq!(&px[0..8], &[1, 2, 0, 0, 0, 0, 2, 1]);
        assert_eq!(&px[8..16], &[0; 8]);
    }

    #[test]
    fn master_palette_covers_64_entries() {
        assert_eq!(MASTER_PALETTE.len(), 64);
        // Spot-check canonical entries: $0F black, $20 near-white, $01 blue.
        assert_eq!(MASTER_PALETTE[0x0F], [0x00, 0x00, 0x00]);
        assert_eq!(MASTER_PALETTE[0x20], [0xF8, 0xF8, 0xF8]);
        assert_eq!(MASTER_PALETTE[0x01], [0x00, 0x00, 0xFC]);
    }
}
