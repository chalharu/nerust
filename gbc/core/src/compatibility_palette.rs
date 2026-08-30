// CGB DMG-compatibility palette selection documented by Pan Docs:
// https://gbdev.io/pandocs/Power_Up_Sequence.html#compatibility-palettes
// The lookup database reproduces the hardware boot hand-off state.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct CompatibilityPalettes {
    pub(crate) bg: [u16; 4],
    pub(crate) obj0: [u16; 4],
    pub(crate) obj1: [u16; 4],
}

const TITLE_CHECKSUMS: [u8; 79] = [
    0x00, 0x88, 0x16, 0x36, 0xD1, 0xDB, 0xF2, 0x3C, 0x8C, 0x92, 0x3D, 0x5C, 0x58, 0xC9, 0x3E, 0x70,
    0x1D, 0x59, 0x69, 0x19, 0x35, 0xA8, 0x14, 0xAA, 0x75, 0x95, 0x99, 0x34, 0x6F, 0x15, 0xFF, 0x97,
    0x4B, 0x90, 0x17, 0x10, 0x39, 0xF7, 0xF6, 0xA2, 0x49, 0x4E, 0x43, 0x68, 0xE0, 0x8B, 0xF0, 0xCE,
    0x0C, 0x29, 0xE8, 0xB7, 0x86, 0x9A, 0x52, 0x01, 0x9D, 0x71, 0x9C, 0xBD, 0x5D, 0x6D, 0x67, 0x3F,
    0x6B, 0xB3, 0x46, 0x28, 0xA5, 0xC6, 0xD3, 0x27, 0x61, 0x18, 0x66, 0x6A, 0xBF, 0x0D, 0xF4,
];

const AMBIGUOUS_FOURTH_LETTERS: [[u8; 14]; 2] = [*b"BEFAARBEKEK R-", *b"URAR INAILICE "];

// (shuffle flags, palette triplet ID), indexed by resolved title ID.
const PALETTE_SPECS: [(u8, u8); 94] = [
    (3, 28),
    (0, 8),
    (0, 18),
    (5, 3),
    (5, 2),
    (0, 7),
    (4, 7),
    (2, 11),
    (1, 0),
    (0, 18),
    (3, 5),
    (5, 8),
    (0, 22),
    (5, 9),
    (4, 6),
    (5, 17),
    (3, 8),
    (5, 0),
    (4, 7),
    (3, 6),
    (0, 18),
    (5, 1),
    (1, 16),
    (1, 28),
    (0, 18),
    (4, 5),
    (0, 18),
    (3, 4),
    (0, 27),
    (0, 7),
    (0, 6),
    (3, 15),
    (3, 14),
    (3, 14),
    (5, 14),
    (5, 15),
    (3, 15),
    (5, 18),
    (5, 15),
    (5, 18),
    (5, 8),
    (5, 11),
    (3, 15),
    (5, 15),
    (4, 6),
    (5, 14),
    (5, 2),
    (5, 2),
    (0, 18),
    (5, 15),
    (0, 19),
    (0, 18),
    (5, 1),
    (3, 14),
    (5, 15),
    (5, 15),
    (5, 13),
    (0, 6),
    (2, 12),
    (3, 14),
    (5, 15),
    (5, 15),
    (0, 18),
    (3, 28),
    (5, 12),
    (5, 8),
    (3, 10),
    (3, 14),
    (0, 19),
    (5, 0),
    (1, 13),
    (5, 8),
    (1, 11),
    (5, 12),
    (3, 4),
    (5, 12),
    (3, 13),
    (4, 7),
    (5, 28),
    (3, 0),
    (5, 20),
    (0, 19),
    (3, 18),
    (3, 28),
    (5, 21),
    (5, 14),
    (5, 14),
    (3, 28),
    (3, 28),
    (3, 5),
    (5, 2),
    (3, 12),
    (3, 4),
    (4, 5),
];

// Byte offsets into the flattened BGR555 palette table.
const PALETTE_OFFSETS: [[u8; 3]; 29] = [
    [16 * 8, 22 * 8, 8 * 8],
    [17 * 8, 4 * 8, 13 * 8],
    [27 * 8 + 6, 0, 14 * 8],
    [27 * 8 + 6, 4 * 8, 15 * 8],
    [4 * 8, 4 * 8, 7 * 8],
    [4 * 8, 22 * 8, 18 * 8],
    [4 * 8, 22 * 8, 20 * 8],
    [28 * 8, 22 * 8, 24 * 8],
    [19 * 8, 22 * 8 + 6, 9 * 8],
    [16 * 8, 28 * 8, 10 * 8],
    [3 * 8 + 6, 3 * 8 + 6, 11 * 8],
    [4 * 8, 23 * 8, 28 * 8],
    [17 * 8, 22 * 8, 2 * 8],
    [4 * 8, 0, 2 * 8],
    [4 * 8, 28 * 8, 3 * 8],
    [28 * 8, 3 * 8, 0],
    [3 * 8, 28 * 8, 4 * 8],
    [21 * 8, 28 * 8, 4 * 8],
    [3 * 8, 28 * 8, 0],
    [4 * 8, 3 * 8, 27 * 8],
    [25 * 8, 3 * 8, 28 * 8],
    [0, 28 * 8, 8 * 8],
    [5 * 8, 5 * 8, 5 * 8],
    [3 * 8, 28 * 8, 12 * 8],
    [4 * 8, 3 * 8, 28 * 8],
    [0, 0, 8],
    [28 * 8, 3 * 8, 6 * 8],
    [26 * 8, 26 * 8, 26 * 8],
    [4 * 8, 28 * 8, 29 * 8],
];

const PALETTES: [u16; 30 * 4] = [
    0x7FFF, 0x32BF, 0x00D0, 0x0000, 0x639F, 0x4279, 0x15B0, 0x04CB, 0x7FFF, 0x6E31, 0x454A, 0x0000,
    0x7FFF, 0x1BEF, 0x0200, 0x0000, 0x7FFF, 0x421F, 0x1CF2, 0x0000, 0x7FFF, 0x5294, 0x294A, 0x0000,
    0x7FFF, 0x03FF, 0x012F, 0x0000, 0x7FFF, 0x03EF, 0x01D6, 0x0000, 0x7FFF, 0x42B5, 0x3DC8, 0x0000,
    0x7E74, 0x03FF, 0x0180, 0x0000, 0x67FF, 0x77AC, 0x1A13, 0x2D6B, 0x7ED6, 0x4BFF, 0x2175, 0x0000,
    0x53FF, 0x4A5F, 0x7E52, 0x0000, 0x4FFF, 0x7ED2, 0x3A4C, 0x1CE0, 0x03ED, 0x7FFF, 0x255F, 0x0000,
    0x036A, 0x021F, 0x03FF, 0x7FFF, 0x7FFF, 0x01DF, 0x0112, 0x0000, 0x231F, 0x035F, 0x00F2, 0x0009,
    0x7FFF, 0x03EA, 0x011F, 0x0000, 0x299F, 0x001A, 0x000C, 0x0000, 0x7FFF, 0x027F, 0x001F, 0x0000,
    0x7FFF, 0x03E0, 0x0206, 0x0120, 0x7FFF, 0x7EEB, 0x001F, 0x7C00, 0x7FFF, 0x3FFF, 0x7E00, 0x001F,
    0x7FFF, 0x03FF, 0x001F, 0x0000, 0x03FF, 0x001F, 0x000C, 0x0000, 0x7FFF, 0x033F, 0x0193, 0x0000,
    0x0000, 0x4200, 0x037F, 0x7FFF, 0x7FFF, 0x7E8C, 0x7C00, 0x0000, 0x7FFF, 0x1BEF, 0x6180, 0x0000,
];

pub(crate) fn select(rom: &[u8]) -> CompatibilityPalettes {
    let spec_index = palette_spec_index(rom);
    let (flags, triplet_id) = PALETTE_SPECS[spec_index];
    let offsets = PALETTE_OFFSETS[usize::from(triplet_id)];
    let bg_offset = offsets[2];
    let obj0_offset = if flags & 0b001 != 0 {
        offsets[0]
    } else {
        bg_offset
    };
    let obj1_offset = if flags & 0b100 != 0 {
        offsets[1]
    } else if flags & 0b010 != 0 {
        offsets[0]
    } else {
        bg_offset
    };
    CompatibilityPalettes {
        bg: palette_at(bg_offset),
        obj0: palette_at(obj0_offset),
        obj1: palette_at(obj1_offset),
    }
}

fn palette_spec_index(rom: &[u8]) -> usize {
    if !is_nintendo_licensee(rom) {
        return 0;
    }
    let Some(title) = rom.get(0x0134..=0x0143) else {
        return 0;
    };
    let checksum = title.iter().fold(0u8, |sum, byte| sum.wrapping_add(*byte));
    let Some(index) = TITLE_CHECKSUMS.iter().position(|value| *value == checksum) else {
        return 0;
    };
    if index < 65 {
        return index;
    }

    let column = index - 65;
    let fourth = title[3];
    for (row, letters) in AMBIGUOUS_FOURTH_LETTERS.iter().enumerate() {
        if letters[column] == fourth {
            return index + row * 14;
        }
    }
    if column == 0 && fourth == b'R' {
        return index + 28;
    }
    0
}

fn is_nintendo_licensee(rom: &[u8]) -> bool {
    match rom.get(0x014B) {
        Some(0x33) => rom.get(0x0144..=0x0145) == Some(b"01"),
        Some(0x01) => true,
        _ => false,
    }
}

fn palette_at(byte_offset: u8) -> [u16; 4] {
    let start = usize::from(byte_offset) / 2;
    PALETTES[start..start + 4]
        .try_into()
        .expect("compatibility palette offset must contain four colors")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rom_with_title_checksum(checksum: u8, fourth: u8) -> Vec<u8> {
        let mut rom = vec![0; 0x150];
        rom[0x0134] = checksum.wrapping_sub(fourth);
        rom[0x0137] = fourth;
        rom[0x014B] = 0x01;
        rom
    }

    #[test]
    fn non_nintendo_rom_uses_default_palette_spec() {
        assert_eq!(palette_spec_index(&vec![0; 0x150]), 0);
    }

    #[test]
    fn known_checksum_selects_matching_palette_spec() {
        assert_eq!(palette_spec_index(&rom_with_title_checksum(0x88, 0)), 1);
        let selected = select(&rom_with_title_checksum(0x88, 0));
        assert_eq!(selected.bg, [0x7E74, 0x03FF, 0x0180, 0x0000]);
        assert_eq!(selected.obj0, selected.bg);
        assert_eq!(selected.obj1, selected.bg);
    }

    #[test]
    fn ambiguous_checksum_uses_fourth_title_character() {
        assert_eq!(palette_spec_index(&rom_with_title_checksum(0xB3, b'B')), 65);
        assert_eq!(palette_spec_index(&rom_with_title_checksum(0xB3, b'U')), 79);
        assert_eq!(palette_spec_index(&rom_with_title_checksum(0xB3, b'R')), 93);
        assert_eq!(palette_spec_index(&rom_with_title_checksum(0xB3, b'?')), 0);
    }

    #[test]
    fn new_nintendo_licensee_is_accepted() {
        let mut rom = rom_with_title_checksum(0x88, 0);
        rom[0x014B] = 0x33;
        rom[0x0144..=0x0145].copy_from_slice(b"01");
        assert_eq!(palette_spec_index(&rom), 1);
    }

    #[test]
    fn pokemon_red_uses_assigned_red_palette() {
        let mut rom = vec![0; 0x150];
        rom[0x0134..0x013F].copy_from_slice(b"POKEMON RED");
        rom[0x014B] = 0x01;

        assert_eq!(palette_spec_index(&rom), 22);
        let selected = select(&rom);
        assert_eq!(selected.bg, [0x7FFF, 0x421F, 0x1CF2, 0x0000]);
    }
}
