/// Cartridge type byte ($0147).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CartridgeType {
    RomOnly,
    Mbc1,
    Mbc1Ram,
    Mbc1RamBattery,
    Mbc2,
    Mbc2Battery,
    Mbc3TimerBattery,
    Mbc3TimerRamBattery,
    Mbc3,
    Mbc3Ram,
    Mbc3RamBattery,
    Mbc5,
    Mbc5Ram,
    Mbc5RamBattery,
    Mbc5Rumble,
    Mbc5RumbleRam,
    Mbc5RumbleRamBattery,
    Mbc6,
}

impl CartridgeType {
    pub fn from_byte(v: u8) -> Option<Self> {
        match v {
            0x00 => Some(Self::RomOnly),
            0x01 => Some(Self::Mbc1),
            0x02 => Some(Self::Mbc1Ram),
            0x03 => Some(Self::Mbc1RamBattery),
            0x05 => Some(Self::Mbc2),
            0x06 => Some(Self::Mbc2Battery),
            0x0F => Some(Self::Mbc3TimerBattery),
            0x10 => Some(Self::Mbc3TimerRamBattery),
            0x11 => Some(Self::Mbc3),
            0x12 => Some(Self::Mbc3Ram),
            0x13 => Some(Self::Mbc3RamBattery),
            0x19 => Some(Self::Mbc5),
            0x1A => Some(Self::Mbc5Ram),
            0x1B => Some(Self::Mbc5RamBattery),
            0x1C => Some(Self::Mbc5Rumble),
            0x1D => Some(Self::Mbc5RumbleRam),
            0x1E => Some(Self::Mbc5RumbleRamBattery),
            0x20 => Some(Self::Mbc6),
            _ => None,
        }
    }

    pub fn has_battery(self) -> bool {
        matches!(
            self,
            Self::Mbc1RamBattery
                | Self::Mbc2Battery
                | Self::Mbc3TimerBattery
                | Self::Mbc3TimerRamBattery
                | Self::Mbc3RamBattery
                | Self::Mbc5RamBattery
                | Self::Mbc5RumbleRamBattery
                | Self::Mbc6
        )
    }

    pub fn has_ram(self) -> bool {
        matches!(
            self,
            Self::Mbc1Ram
                | Self::Mbc1RamBattery
                | Self::Mbc2
                | Self::Mbc2Battery
                | Self::Mbc3TimerRamBattery
                | Self::Mbc3Ram
                | Self::Mbc3RamBattery
                | Self::Mbc5Ram
                | Self::Mbc5RamBattery
                | Self::Mbc5RumbleRam
                | Self::Mbc5RumbleRamBattery
                | Self::Mbc6
        )
    }

    pub fn has_rumble(self) -> bool {
        matches!(
            self,
            Self::Mbc5Rumble | Self::Mbc5RumbleRam | Self::Mbc5RumbleRamBattery
        )
    }

    pub fn has_rtc(self) -> bool {
        matches!(self, Self::Mbc3TimerBattery | Self::Mbc3TimerRamBattery)
    }
}

/// ROM size byte ($0148).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RomSize {
    pub bytes: usize,
    pub banks: usize,
}

impl RomSize {
    pub fn from_byte(v: u8) -> Option<Self> {
        let banks = match v {
            0x00 => 2,
            0x01 => 4,
            0x02 => 8,
            0x03 => 16,
            0x04 => 32,
            0x05 => 64,
            0x06 => 128,
            0x07 => 256,
            0x08 => 512,
            _ => return None,
        };
        Some(Self {
            bytes: banks * 0x4000,
            banks,
        })
    }
}

/// RAM size byte ($0149).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RamSize {
    pub bytes: usize,
    pub banks: usize,
}

impl RamSize {
    pub fn from_byte(v: u8) -> Option<Self> {
        match v {
            0x00 => Some(Self { bytes: 0, banks: 0 }),
            0x02 => Some(Self {
                bytes: 0x2000,
                banks: 1,
            }),
            0x03 => Some(Self {
                bytes: 0x8000,
                banks: 4,
            }),
            0x04 => Some(Self {
                bytes: 0x20000,
                banks: 16,
            }),
            0x05 => Some(Self {
                bytes: 0x10000,
                banks: 8,
            }),
            _ => None,
        }
    }
}

/// Parsed cartridge header ($0100-$014F).
#[derive(Debug, Clone)]
pub struct CartridgeHeader {
    pub cartridge_type: CartridgeType,
    pub rom_size: RomSize,
    pub ram_size: RamSize,
    pub cgb_flag: u8,
    pub checksum_valid: bool,
    /// True for 8 Mbit MBC1 multicarts (multiple games with a menu).
    pub multicart: bool,
}

const HEADER_OFFSET: usize = 0x0100;

/// The standard Nintendo logo ($0104-$0133).
const NINTENDO_LOGO: [u8; 0x30] = [
    0xCE, 0xED, 0x66, 0x66, 0xCC, 0x0D, 0x00, 0x0B, 0x03, 0x73, 0x00, 0x83, 0x00, 0x0C, 0x00, 0x0D,
    0x00, 0x08, 0x11, 0x1F, 0x88, 0x89, 0x00, 0x0E, 0xDC, 0xCC, 0x6E, 0xE6, 0xDD, 0xDD, 0xD9, 0x99,
    0xBB, 0xBB, 0x67, 0x63, 0x6E, 0x0E, 0xEC, 0xCC, 0xDD, 0xDC, 0x99, 0x9F, 0xBB, 0xB9, 0x33, 0x3E,
];

pub fn is_supported_rom(rom: &[u8]) -> bool {
    let Some(header) = CartridgeHeader::parse(rom) else {
        return false;
    };
    rom.get(0x0104..0x0134) == Some(NINTENDO_LOGO.as_slice())
        && header.checksum_valid
        && rom.len() >= header.rom_size.bytes
}

#[cfg(test)]
pub(crate) fn finalize_test_rom(rom: &mut [u8]) {
    rom[0x0104..0x0134].copy_from_slice(&NINTENDO_LOGO);
    let mut checksum = 0u8;
    for byte in &rom[0x0134..=0x014C] {
        checksum = checksum.wrapping_sub(*byte).wrapping_sub(1);
    }
    rom[0x014D] = checksum;
}
/// Only 8 Mbit MBC1 multicarts exist. A multicart has at least two games plus
/// a menu, so at least three of the four 2 Mbit pages carry a valid logo.
fn is_mbc1_multicart(rom: &[u8]) -> bool {
    if rom.len() != 0x100000 {
        return false;
    }
    (0..4)
        .filter(|&page| {
            let start = page * 0x40000 + HEADER_OFFSET + 0x04;
            rom.get(start..start + 0x30) == Some(&NINTENDO_LOGO)
        })
        .count()
        >= 3
}

impl CartridgeHeader {
    pub fn parse(rom: &[u8]) -> Option<Self> {
        if rom.len() < HEADER_OFFSET + 0x50 {
            return None;
        }

        let hdr = &rom[HEADER_OFFSET..];

        let cartridge_type = CartridgeType::from_byte(hdr[0x47])?;
        let rom_size = RomSize::from_byte(hdr[0x48])?;
        let ram_size = RamSize::from_byte(hdr[0x49])?;
        let cgb_flag = hdr[0x43];

        let checksum_valid = Self::compute_checksum(hdr);
        let multicart = is_mbc1_multicart(rom);

        Some(Self {
            cartridge_type,
            rom_size,
            ram_size,
            cgb_flag,
            checksum_valid,
            multicart,
        })
    }

    fn compute_checksum(hdr: &[u8]) -> bool {
        let mut checksum: u8 = 0;
        for b in &hdr[0x0034..=0x004C] {
            checksum = checksum.wrapping_sub(*b).wrapping_sub(1);
        }
        checksum == hdr[0x004D]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn minimal_rom() -> Vec<u8> {
        let mut rom = vec![0u8; 0x150];
        // NOP + JP $0150
        rom[0x0100] = 0x00;
        rom[0x0101] = 0xC3;
        rom[0x0102] = 0x50;
        rom[0x0103] = 0x01;
        // Nintendo logo (first 2 bytes for CGB check)
        rom[0x0104] = 0xCE;
        rom[0x0105] = 0xED;
        // Title (zero-filled)
        rom[0x0134..0x0143].fill(0);
        // CGB flag
        rom[0x0143] = 0x80;
        // New licensee
        rom[0x0144] = 0x30;
        rom[0x0145] = 0x31;
        // SGB flag
        rom[0x0146] = 0x00;
        // Cartridge type
        rom[0x0147] = 0x00;
        // ROM size
        rom[0x0148] = 0x00;
        // RAM size
        rom[0x0149] = 0x00;
        // Destination
        rom[0x014A] = 0x00;
        // Old licensee
        rom[0x014B] = 0x33;
        // Mask ROM version
        rom[0x014C] = 0x00;
        // Header checksum
        rom[0x014D] = 0;
        rom
    }

    fn compute_and_set_checksum(rom: &mut [u8]) {
        let mut checksum: u8 = 0;
        for b in &rom[0x0134..=0x014C] {
            checksum = checksum.wrapping_sub(*b).wrapping_sub(1);
        }
        rom[0x014D] = checksum;
    }

    fn supported_rom() -> Vec<u8> {
        let mut rom = vec![0; 0x8000];
        rom[0x0104..0x0134].copy_from_slice(&NINTENDO_LOGO);
        rom[0x0143] = 0x80;
        compute_and_set_checksum(&mut rom);
        rom
    }

    #[test]
    fn supported_rom_requires_logo_checksum_and_declared_length() {
        let rom = supported_rom();
        assert!(is_supported_rom(&rom));

        let mut invalid_logo = rom.clone();
        invalid_logo[0x0104] ^= 0xFF;
        assert!(!is_supported_rom(&invalid_logo));

        let mut invalid_checksum = rom.clone();
        invalid_checksum[0x014D] ^= 0xFF;
        assert!(!is_supported_rom(&invalid_checksum));

        assert!(!is_supported_rom(&rom[..0x4000]));
    }
    #[test]
    fn parse_rom_only() {
        let mut rom = minimal_rom();
        rom[0x0147] = 0x00;
        compute_and_set_checksum(&mut rom);
        let header = CartridgeHeader::parse(&rom).expect("parse");
        assert_eq!(header.cartridge_type, CartridgeType::RomOnly);
    }

    #[test]
    fn parse_mbc1_ram_battery() {
        let mut rom = minimal_rom();
        rom[0x0147] = 0x03;
        rom[0x0148] = 0x04;
        rom[0x0149] = 0x03;
        compute_and_set_checksum(&mut rom);
        let header = CartridgeHeader::parse(&rom).expect("parse");
        assert_eq!(header.cartridge_type, CartridgeType::Mbc1RamBattery);
        assert_eq!(header.rom_size.banks, 32);
        assert_eq!(header.ram_size.banks, 4);
        assert!(header.cartridge_type.has_battery());
    }

    #[test]
    fn cgb_flag_0x80_is_enhanced() {
        let mut rom = minimal_rom();
        rom[0x0143] = 0x80;
        compute_and_set_checksum(&mut rom);
        let header = CartridgeHeader::parse(&rom).expect("parse");
        assert_eq!(header.cgb_flag, 0x80);
    }

    #[test]
    fn invalid_checksum_is_detected() {
        let mut rom = minimal_rom();
        compute_and_set_checksum(&mut rom);
        rom[0x014D] = rom[0x014D].wrapping_add(1);
        let header = CartridgeHeader::parse(&rom).expect("parse");
        assert!(!header.checksum_valid);
    }

    #[test]
    fn rom_too_short_returns_none() {
        let rom = vec![0u8; 0x100];
        assert!(CartridgeHeader::parse(&rom).is_none());
    }

    #[test]
    fn unknown_cartridge_type_returns_none() {
        let mut rom = minimal_rom();
        rom[0x0147] = 0xFF;
        compute_and_set_checksum(&mut rom);
        assert!(CartridgeHeader::parse(&rom).is_none());
    }

    #[test]
    fn cartridge_type_has_ram() {
        assert!(CartridgeType::Mbc1Ram.has_ram());
        assert!(!CartridgeType::RomOnly.has_ram());
        assert!(!CartridgeType::Mbc1.has_ram());
    }

    #[test]
    fn cartridge_type_has_battery() {
        assert!(CartridgeType::Mbc1RamBattery.has_battery());
        assert!(!CartridgeType::Mbc1Ram.has_battery());
    }

    #[test]
    fn cartridge_type_has_rumble() {
        assert!(CartridgeType::Mbc5RumbleRam.has_rumble());
        assert!(!CartridgeType::Mbc5Ram.has_rumble());
    }

    #[test]
    fn timer_only_mbc3_has_rtc_but_no_ram() {
        assert!(CartridgeType::Mbc3TimerBattery.has_rtc());
        assert!(!CartridgeType::Mbc3TimerBattery.has_ram());
        assert!(CartridgeType::Mbc3TimerBattery.has_battery());
    }

    #[test]
    fn mbc6_has_expected_capabilities() {
        let mbc6 = CartridgeType::from_byte(0x20).unwrap();
        assert_eq!(mbc6, CartridgeType::Mbc6);
        assert!(mbc6.has_ram());
        assert!(mbc6.has_battery());
        assert!(!mbc6.has_rumble());
    }

    #[test]
    fn rom_size_max_banks() {
        let size = RomSize::from_byte(0x08).expect("512 banks");
        assert_eq!(size.banks, 512);
        assert_eq!(size.bytes, 512 * 0x4000);
        assert!(RomSize::from_byte(0x09).is_none());
    }

    #[test]
    fn ram_size_0x05_is_64kib() {
        let size = RamSize::from_byte(0x05).expect("8 banks");
        assert_eq!(size.banks, 8);
        assert_eq!(size.bytes, 0x10000);
    }
}
