use crate::cartridge_header::{CartridgeHeader, CartridgeType};
use crc::{CRC_32_ISO_HDLC, Crc};

const MBC_BANK_SIZE: usize = 0x4000;
const MENU_SIZE: usize = 0x8000;
const MAX_MMM01_ROM_SIZE: usize = 0x800000;
const MAX_M161_ROM_SIZE: usize = 8 * MENU_SIZE;
const MAX_WISDOM_TREE_ROM_SIZE: usize = 64 * MENU_SIZE;
const M161_HEADER_CRC32: u32 = 0xA61F_3EE1;
const CRC32: Crc<u32> = Crc::<u32>::new(&CRC_32_ISO_HDLC);

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

    let mapper = detect_unlicensed_mapper(rom, &header)
        .unwrap_or(DetectedMapper::Header(header.cartridge_type));
    Some(CartridgeDescriptor {
        mapper,
        header,
        header_offset: 0,
        initial_rom0_bank: 0,
        initial_romx_bank: 1,
    })
}

fn detect_unlicensed_mapper(rom: &[u8], header: &CartridgeHeader) -> Option<DetectedMapper> {
    if header.cartridge_type != CartridgeType::RomOnly {
        return None;
    }
    if is_m161(rom) {
        Some(DetectedMapper::M161)
    } else if is_wisdom_tree(rom) {
        Some(DetectedMapper::WisdomTree)
    } else {
        None
    }
}

fn is_m161(rom: &[u8]) -> bool {
    rom.len() >= 2 * MENU_SIZE
        && rom.len() <= MAX_M161_ROM_SIZE
        && rom.len().is_multiple_of(MENU_SIZE)
        && rom
            .get(0x0100..0x0150)
            .is_some_and(|header| is_known_m161_header(CRC32.checksum(header)))
}

fn is_known_m161_header(header_crc32: u32) -> bool {
    header_crc32 == M161_HEADER_CRC32
}

fn is_wisdom_tree(rom: &[u8]) -> bool {
    rom.len() >= MENU_SIZE
        && rom.len() <= MAX_WISDOM_TREE_ROM_SIZE
        && rom.len().is_multiple_of(MENU_SIZE)
        && rom.get(0x00F0..0x0100).is_some_and(all_zero)
        && rom.get(0x0134..0x014C).is_some_and(all_zero)
        && rom.get(0x014D) == Some(&0xE7)
        && rom
            .get(0x0300..)
            .is_some_and(|body| body.windows(11).any(is_wisdom_tree_signature))
}

fn all_zero(bytes: &[u8]) -> bool {
    bytes.iter().all(|byte| *byte == 0)
}

fn is_wisdom_tree_signature(window: &[u8]) -> bool {
    window.get(..6) == Some(b"WISDOM") && window.get(7..) == Some(b"TREE")
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
        assert_eq!(
            descriptor.mapper,
            DetectedMapper::Header(CartridgeType::RomOnly)
        );
        assert_eq!(
            (descriptor.initial_rom0_bank, descriptor.initial_romx_bank),
            (0, 1)
        );
    }

    #[test]
    fn invalid_standard_header_is_rejected() {
        let rom = standard_rom(0x00, 0);

        let mut invalid_logo = rom.clone();
        invalid_logo[0x0104] ^= 0xFF;
        assert!(detect_cartridge(&invalid_logo).is_none());

        let mut invalid_checksum = rom.clone();
        invalid_checksum[0x014D] ^= 0xFF;
        assert!(detect_cartridge(&invalid_checksum).is_none());

        assert!(detect_cartridge(&rom[..0x4000]).is_none());
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
        assert_eq!(
            descriptor.mapper,
            DetectedMapper::Header(CartridgeType::Mmm01)
        );
        assert_eq!(
            (descriptor.initial_rom0_bank, descriptor.initial_romx_bank),
            (6, 7)
        );
        assert!(!descriptor.header.multicart);
    }

    #[test]
    fn m161_known_header_crc_uses_standard_crc32() {
        assert_eq!(CRC32.checksum(b"123456789"), 0xCBF4_3926);
        assert!(is_known_m161_header(0xA61F_3EE1));
        assert!(!is_known_m161_header(0xA61F_3EE0));
    }

    #[test]
    fn wisdom_tree_requires_the_complete_compound_signature() {
        let mut rom = standard_rom(0x00, 1);
        rom[0x0148] = 0;
        rom[0x0300..0x030B].copy_from_slice(b"WISDOM TREE");
        finalize_test_rom(&mut rom);
        assert_eq!(rom[0x014D], 0xE7);
        assert_eq!(
            detect_cartridge(&rom).unwrap().mapper,
            DetectedMapper::WisdomTree
        );

        for offset in [0x00F0, 0x0134, 0x014D, 0x0300] {
            let mut invalid = rom.clone();
            invalid[offset] ^= 1;
            assert_ne!(
                detect_cartridge(&invalid).map(|value| value.mapper),
                Some(DetectedMapper::WisdomTree)
            );
        }
    }
}
