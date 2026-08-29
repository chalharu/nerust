use crc::{CRC_64_XZ, Crc};
use nerust_core_traits::identity::SystemIdentity;

use crate::{
    cartridge_descriptor::{CartridgeDescriptor, detect_cartridge},
    cartridge_header::HEADER_OFFSET,
};

const CRC64: Crc<u64> = Crc::<u64>::new(&CRC_64_XZ);

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct GbcRomIdentity {
    pub cartridge_type: u8,
    pub cgb_flag: u8,
    pub rom_len: usize,
    pub declared_rom_len: usize,
    pub ram_len: usize,
    pub has_battery: bool,
    pub has_rtc: bool,
    pub multicart: bool,
    pub rom_crc64: u64,
}

impl GbcRomIdentity {
    pub fn from_rom(rom: &[u8]) -> Option<Self> {
        let descriptor = detect_cartridge(rom)?;
        Self::from_descriptor(rom, &descriptor)
    }

    pub fn from_descriptor(rom: &[u8], descriptor: &CartridgeDescriptor) -> Option<Self> {
        let header = &descriptor.header;
        let type_offset = descriptor.header_offset.checked_add(HEADER_OFFSET + 0x47)?;
        Some(Self {
            cartridge_type: *rom.get(type_offset)?,
            cgb_flag: header.cgb_flag,
            rom_len: rom.len(),
            declared_rom_len: header.rom_size.bytes,
            ram_len: header.ram_size.bytes,
            has_battery: header.cartridge_type.has_battery(),
            has_rtc: header.cartridge_type.has_rtc(),
            multicart: header.multicart,
            rom_crc64: CRC64.checksum(rom),
        })
    }

    pub fn into_system_identity(self) -> Result<SystemIdentity, rmp_serde::encode::Error> {
        Ok(SystemIdentity::new(
            Box::new(GbcSystemId),
            rmp_serde::to_vec_named(&self)?,
        ))
    }
}

nerust_core_traits::declare_system_id!(pub GbcSystemId, "gbc");

#[cfg(test)]
mod tests {
    use super::*;

    fn rom(fill: u8) -> Vec<u8> {
        let mut rom = vec![fill; 0x8000];
        rom[0x0143] = 0x80;
        rom[0x0147] = 0x00;
        rom[0x0148] = 0x00;
        rom[0x0149] = 0x00;
        rom
    }

    #[test]
    fn body_changes_identity_even_with_same_header() {
        let first = GbcRomIdentity::from_rom(&rom(0)).unwrap();
        let mut second_rom = rom(0);
        second_rom[0x2000] = 1;
        let second = GbcRomIdentity::from_rom(&second_rom).unwrap();
        assert_ne!(first.rom_crc64, second.rom_crc64);
    }

    #[test]
    fn converts_to_gbc_system_identity() {
        let identity = GbcRomIdentity::from_rom(&rom(0))
            .unwrap()
            .into_system_identity()
            .unwrap();
        assert_eq!(identity.system_id.to_string(), "gbc");
        assert!(!identity.identity_bytes.is_empty());
    }
}
