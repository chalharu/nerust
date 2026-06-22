use nerust_nes_core::cartridge_rom::CartridgeData;
use nerust_nes_core::rom_parse;

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum CartridgeParseError {
    #[error("data integrity error in data")]
    DataError,
    #[error("file ends unexpectedly")]
    UnexpectedEof,
    #[allow(dead_code)]
    #[error("unexpected error")]
    Unexpected,
}

pub fn parse_cartridge_bytes(data: &[u8]) -> Result<CartridgeData, CartridgeParseError> {
    rom_parse::parse_rom(data).map_err(|e| match e {
        nerust_nes_core::cartridge_error::CartridgeError::DataError => {
            CartridgeParseError::DataError
        }
        nerust_nes_core::cartridge_error::CartridgeError::UnexpectedEof => {
            CartridgeParseError::UnexpectedEof
        }
        nerust_nes_core::cartridge_error::CartridgeError::Unexpected => {
            CartridgeParseError::Unexpected
        }
    })
}

#[cfg(test)]
mod tests {
    use super::parse_cartridge_bytes;
    use nerust_nes_core::mirror::MirrorMode;
    use nerust_nes_core::rom_format::RomFormat;

    #[test]
    fn parses_ines_metadata() {
        let mut rom = vec![
            0x4E, 0x45, 0x53, 0x1A, 0x02, 0x01, 0x41, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00,
        ];
        rom.resize(16 + 0x8000 + 0x2000, 0);

        let data = parse_cartridge_bytes(&rom).expect("rom should parse");

        assert_eq!(data.format(), RomFormat::INes);
        assert_eq!(data.mapper_type(), 4);
        assert_eq!(data.sub_mapper_type(), 0);
        assert_eq!(data.mirror_mode(), MirrorMode::Vertical);
        assert!(!data.has_battery());
        assert_eq!(data.prog_rom_len(), 0x8000);
        assert_eq!(data.char_rom_len(), 0x2000);
    }

    #[test]
    fn parses_nes20_memory_sizes() {
        let mut rom = vec![
            0x4E, 0x45, 0x53, 0x1A, 0x02, 0x00, 0x08, 0x08, 0x30, 0x00, 0x54, 0x76, 0x00, 0x00,
            0x00, 0x00,
        ];
        rom.resize(16 + 0x8000, 0);

        let data = parse_cartridge_bytes(&rom).expect("rom should parse");

        assert_eq!(data.format(), RomFormat::Nes20);
        assert_eq!(data.mapper_type(), 0);
        assert_eq!(data.sub_mapper_type(), 3);
        assert_eq!(data.mirror_mode(), MirrorMode::Single0);
        assert_eq!(data.pram_length(), 1 << (6 + 4));
        assert_eq!(data.save_pram_length(), 1 << (6 + 5));
        assert_eq!(data.vram_length(), 1 << (6 + 6));
        assert_eq!(data.save_vram_length(), 1 << (6 + 7));
    }

    #[test]
    fn nes20_chr_ram_sizes_do_not_add_implicit_extra_bank() {
        let mut rom = vec![
            0x4E, 0x45, 0x53, 0x1A, 0x04, 0x00, 0xB1, 0x08, 0x00, 0x00, 0x00, 0x09, 0x00, 0x00,
            0x00, 0x00,
        ];
        rom.resize(16 + 0x10000, 0);

        let data = parse_cartridge_bytes(&rom).expect("rom should parse");

        assert_eq!(data.char_rom_len(), 0);
        assert_eq!(data.vram_length(), 1 << (6 + 9));
        assert_eq!(data.save_vram_length(), 0);
    }

    #[test]
    fn nes20_chr_nvram_only_does_not_inject_implicit_chr_ram() {
        let mut rom = vec![
            0x4E, 0x45, 0x53, 0x1A, 0x04, 0x00, 0x08, 0x08, 0x00, 0x00, 0x00, 0x90, 0x00, 0x00,
            0x00, 0x00,
        ];
        rom.resize(16 + 0x10000, 0);

        let data = parse_cartridge_bytes(&rom).expect("rom should parse");

        assert_eq!(data.char_rom_len(), 0);
        assert_eq!(data.vram_length(), 0);
        assert_eq!(data.save_vram_length(), 1 << (6 + 9));
    }
}
