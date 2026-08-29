use crate::cartridge_header::{CartridgeHeader, CartridgeType};

const MBC_BANK_SIZE: usize = 0x4000;
const MENU_SIZE: usize = 0x8000;
const MAX_MMM01_ROM_SIZE: usize = 0x800000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DetectedMapper {
    Header(CartridgeType),
    M161,
    WisdomTree,
}

#[derive(Debug, Clone)]
pub struct CartridgeDescriptor {
    pub header: CartridgeHeader,
    pub header_offset: usize,
    pub mapper: DetectedMapper,
    pub initial_rom0_bank: usize,
    pub initial_romx_bank: usize,
}

pub fn detect_cartridge(rom: &[u8]) -> Option<CartridgeDescriptor> {
    if let Some(descriptor) = detect_mmm01(rom) {
        return Some(descriptor);
    }

    let header = validated_header(rom, 0)?;
    if rom.len() < header.rom_size.bytes {
        return None;
    }

    Some(CartridgeDescriptor {
        mapper: DetectedMapper::Header(header.cartridge_type),
        header,
        header_offset: 0,
        initial_rom0_bank: 0,
        initial_romx_bank: 1,
    })
}

fn detect_mmm01(rom: &[u8]) -> Option<CartridgeDescriptor> {
    if rom.len() < MENU_SIZE
        || rom.len() > MAX_MMM01_ROM_SIZE
        || !rom.len().is_multiple_of(MENU_SIZE)
    {
        return None;
    }

    let header_offset = rom.len() - MENU_SIZE;
    let header = validated_header(rom, header_offset)?;
    if !matches!(
        header.cartridge_type,
        CartridgeType::Mmm01 | CartridgeType::Mmm01Ram | CartridgeType::Mmm01RamBattery
    ) || header.rom_size.bytes != rom.len()
    {
        return None;
    }

    let bank_count = rom.len() / MBC_BANK_SIZE;
    Some(CartridgeDescriptor {
        mapper: DetectedMapper::Header(header.cartridge_type),
        header,
        header_offset,
        initial_rom0_bank: bank_count - 2,
        initial_romx_bank: bank_count - 1,
    })
}

fn validated_header(rom: &[u8], base_offset: usize) -> Option<CartridgeHeader> {
    let header = CartridgeHeader::parse_at(rom, base_offset)?;
    (header.checksum_valid && CartridgeHeader::has_valid_logo(rom, base_offset)).then_some(header)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cartridge_header::finalize_test_rom;

    fn standard_rom(cartridge_type: u8, rom_size: u8) -> Vec<u8> {
        let mut rom = vec![0; 0x8000 << rom_size];
        rom[0x0147] = cartridge_type;
        rom[0x0148] = rom_size;
        finalize_test_rom(&mut rom);
        rom
    }

    #[test]
    fn standard_rom_uses_leading_header_and_initial_banks() {
        let rom = standard_rom(0x00, 0);
        let descriptor = detect_cartridge(&rom).unwrap();
        assert_eq!(descriptor.header_offset, 0);
        assert_eq!(descriptor.mapper, DetectedMapper::Header(CartridgeType::RomOnly));
        assert_eq!((descriptor.initial_rom0_bank, descriptor.initial_romx_bank), (0, 1));
    }

    #[test]
    fn invalid_standard_header_is_rejected() {
        let mut rom = standard_rom(0x00, 0);
        rom[0x0104] ^= 0xFF;
        assert!(detect_cartridge(&rom).is_none());
    }

    #[test]
    fn mmm01_uses_trailing_header_even_when_leading_game_header_is_valid() {
        let mut rom = standard_rom(0x00, 2);
        let menu_base = rom.len() - MENU_SIZE;
        rom[menu_base + 0x0147] = 0x0B;
        rom[menu_base + 0x0148] = 2;
        finalize_test_rom(&mut rom[menu_base..]);

        let descriptor = detect_cartridge(&rom).unwrap();
        assert_eq!(descriptor.header_offset, menu_base);
        assert_eq!(descriptor.mapper, DetectedMapper::Header(CartridgeType::Mmm01));
        assert_eq!((descriptor.initial_rom0_bank, descriptor.initial_romx_bank), (6, 7));
    }
}