const HEADER_SIZE: usize = 0xC0;
const LOGO_OFFSET: usize = 0x04;
const LOGO_SIZE: usize = 156;

/// GBA Nintendo logo (compressed bitmap). Must match BIOS copy.
const NINTENDO_LOGO_GBA: [u8; LOGO_SIZE] = [
    0x24, 0xFF, 0xAE, 0x51, 0x69, 0x9A, 0xA2, 0x21, 0x3D, 0x84, 0x82, 0x0A, 0x84, 0xE4, 0x09, 0xAD,
    0x11, 0x24, 0x8B, 0x98, 0xC0, 0x81, 0x7F, 0x21, 0xA3, 0x52, 0xBE, 0x19, 0x93, 0x09, 0xCE, 0x20,
    0x10, 0x46, 0x4A, 0x4A, 0xF8, 0x27, 0x31, 0xEC, 0x58, 0xC7, 0xE8, 0x33, 0x82, 0xE3, 0xCE, 0xBF,
    0x85, 0xF4, 0xDF, 0x94, 0xCE, 0x4B, 0x09, 0xC1, 0x94, 0x56, 0x8A, 0xC0, 0x13, 0x72, 0xA7, 0xFC,
    0x9F, 0x84, 0x4D, 0x73, 0xA3, 0xCA, 0x9A, 0x61, 0x58, 0x97, 0xA3, 0x27, 0xFC, 0x03, 0x98, 0x76,
    0x23, 0x1D, 0xC7, 0x61, 0x03, 0x04, 0xAE, 0x56, 0xBF, 0x38, 0x84, 0x00, 0x40, 0xA0, 0x0E, 0xFD,
    0xFF, 0x52, 0xFE, 0x03, 0x6F, 0x95, 0x30, 0xF1, 0x97, 0xFB, 0xC0, 0x85, 0x60, 0xD6, 0x80, 0x25,
    0xA9, 0x63, 0xBE, 0x03, 0x01, 0x4E, 0x38, 0xE2, 0xF9, 0xA2, 0x34, 0xFF, 0xBB, 0x3E, 0x03, 0x44,
    0x78, 0x00, 0x90, 0xCB, 0x88, 0x11, 0x3A, 0x94, 0x65, 0xC0, 0x7C, 0x63, 0x87, 0xF0, 0x3C, 0xAF,
    0xD6, 0x25, 0xE4, 0x8B, 0x38, 0x0A, 0xAC, 0x72, 0x21, 0xD4, 0xF8, 0x07,
];

#[derive(Debug, Clone)]
pub struct GbaHeader {
    pub entry_point: u32,
    pub logo_valid: bool,
    pub title: [u8; 12],
    pub game_code: [u8; 4],
    pub maker_code: [u8; 2],
    pub fixed_valid: bool,
    pub complement_valid: bool,
    pub software_version: u8,
}

impl GbaHeader {
    pub fn parse(rom: &[u8]) -> Option<Self> {
        if rom.len() < HEADER_SIZE {
            return None;
        }
        let entry_point = u32::from_le_bytes([rom[0x00], rom[0x01], rom[0x02], rom[0x03]]);
        let logo_valid = rom[LOGO_OFFSET..LOGO_OFFSET + LOGO_SIZE] == NINTENDO_LOGO_GBA;
        let mut title = [0u8; 12];
        title.copy_from_slice(&rom[0xA0..0xAC]);
        let mut game_code = [0u8; 4];
        game_code.copy_from_slice(&rom[0xAC..0xB0]);
        let mut maker_code = [0u8; 2];
        maker_code.copy_from_slice(&rom[0xB0..0xB2]);
        let fixed_valid = rom[0xB2] == 0x96;
        let software_version = rom[0xBC];
        let complement_valid = Self::complement_check(rom);
        Some(Self {
            entry_point,
            logo_valid,
            title,
            game_code,
            maker_code,
            fixed_valid,
            complement_valid,
            software_version,
        })
    }

    pub fn has_valid_logo(rom: &[u8]) -> bool {
        if rom.len() < LOGO_OFFSET + LOGO_SIZE {
            return false;
        }
        rom[LOGO_OFFSET..LOGO_OFFSET + LOGO_SIZE] == NINTENDO_LOGO_GBA
    }

    fn complement_check(rom: &[u8]) -> bool {
        let mut chk: u8 = 0;
        for &b in &rom[0xA0..0xBC] {
            chk = chk.wrapping_sub(b).wrapping_sub(1);
        }
        chk = chk.wrapping_sub(0x19);
        chk == rom[0xBD]
    }
}

/// Test helper: fill logo and complement check for a ROM buffer.
pub fn finalize_test_gba_rom(rom: &mut [u8]) {
    if rom.len() < HEADER_SIZE {
        return;
    }
    rom[LOGO_OFFSET..LOGO_OFFSET + LOGO_SIZE].copy_from_slice(&NINTENDO_LOGO_GBA);
    rom[0xB2] = 0x96;
    let mut chk: u8 = 0;
    for &b in &rom[0xA0..0xBC] {
        chk = chk.wrapping_sub(b).wrapping_sub(1);
    }
    chk = chk.wrapping_sub(0x19);
    rom[0xBD] = chk;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_valid_header() {
        let mut rom = vec![0u8; 0xC0];
        rom[0xB2] = 0x96;
        rom[0xA0..0xAC].copy_from_slice(b"POKEMON EMER");
        rom[0xAC..0xB0].copy_from_slice(b"BPEE");
        rom[0xB0..0xB2].copy_from_slice(b"01");
        finalize_test_gba_rom(&mut rom);
        let h = GbaHeader::parse(&rom).unwrap();
        assert!(h.logo_valid);
        assert!(h.fixed_valid);
        assert!(h.complement_valid);
        assert_eq!(&h.title, b"POKEMON EMER");
    }

    #[test]
    fn parse_rejects_bad_logo() {
        let mut rom = vec![0u8; 0xC0];
        rom[0xB2] = 0x96;
        finalize_test_gba_rom(&mut rom);
        rom[0x04] ^= 0xFF;
        let h = GbaHeader::parse(&rom).unwrap();
        assert!(!h.logo_valid);
    }

    #[test]
    fn parse_rejects_bad_fixed() {
        let mut rom = vec![0u8; 0xC0];
        finalize_test_gba_rom(&mut rom);
        rom[0xB2] = 0x00;
        let h = GbaHeader::parse(&rom).unwrap();
        assert!(!h.fixed_valid);
    }

    #[test]
    fn complement_check() {
        let mut rom = vec![0u8; 0xC0];
        finalize_test_gba_rom(&mut rom);
        let h = GbaHeader::parse(&rom).unwrap();
        assert!(h.complement_valid);
        rom[0xA0] ^= 0x01;
        let h2 = GbaHeader::parse(&rom).unwrap();
        assert!(!h2.complement_valid);
    }

    #[test]
    fn finalize_test_gba_rom_fills_logo_and_complement() {
        let mut rom = vec![0u8; 0x200];
        finalize_test_gba_rom(&mut rom);
        assert!(GbaHeader::has_valid_logo(&rom));
        assert!(GbaHeader::parse(&rom).unwrap().complement_valid);
    }
}
