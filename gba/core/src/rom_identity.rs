use crc::{CRC_64_XZ, Crc};
use serde::{Deserialize, Serialize};

use crate::cartridge::header::GbaHeader;
use crate::cartridge::save::SaveType;

nerust_core_traits::declare_system_id!(pub GbaSystemId, "gba");

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GbaRomIdentity {
    pub title: String,
    pub game_code: String,
    pub maker_code: String,
    pub fixed_valid: bool,
    pub complement_valid: bool,
    pub logo_valid: bool,
    pub save_type: SaveTypeSer,
    pub rom_len: usize,
    pub rom_crc64: u64,
}

/// Serializable wrapper for SaveType
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum SaveTypeSer {
    None,
    Eeprom512,
    Eeprom8k,
    Sram,
    Flash64,
    Flash128,
}

impl From<SaveType> for SaveTypeSer {
    fn from(v: SaveType) -> Self {
        match v {
            SaveType::None => Self::None,
            SaveType::Eeprom512 => Self::Eeprom512,
            SaveType::Eeprom8k => Self::Eeprom8k,
            SaveType::Sram => Self::Sram,
            SaveType::Flash64 => Self::Flash64,
            SaveType::Flash128 => Self::Flash128,
        }
    }
}

impl GbaRomIdentity {
    pub fn from_rom(rom: &[u8]) -> Option<Self> {
        let header = GbaHeader::parse(rom)?;
        let save_type = crate::cartridge::save::detect_save_type(rom);
        let crc64 = Crc::<u64>::new(&CRC_64_XZ).checksum(rom);
        let title = String::from_utf8_lossy(&header.title)
            .trim_matches('\0')
            .trim()
            .to_string();
        let game_code = String::from_utf8_lossy(&header.game_code).to_string();
        let maker_code = String::from_utf8_lossy(&header.maker_code).to_string();
        Some(Self {
            title,
            game_code,
            maker_code,
            fixed_valid: header.fixed_valid,
            complement_valid: header.complement_valid,
            logo_valid: header.logo_valid,
            save_type: save_type.into(),
            rom_len: rom.len(),
            rom_crc64: crc64,
        })
    }

    pub fn into_system_identity(
        self,
    ) -> Result<nerust_core_traits::identity::SystemIdentity, String> {
        let bytes = rmp_serde::to_vec_named(&self).map_err(|e| e.to_string())?;
        Ok(nerust_core_traits::identity::SystemIdentity {
            system_id: Box::new(GbaSystemId),
            identity_bytes: bytes,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cartridge::header::finalize_test_gba_rom;

    #[test]
    fn from_rom_parses_title_and_save() {
        let mut rom = vec![0u8; 0x1000];
        rom[0xA0..0xAC].copy_from_slice(b"POKEMON EMER");
        rom[0xAC..0xB0].copy_from_slice(b"BPEE");
        rom[0xB0..0xB2].copy_from_slice(b"01");
        finalize_test_gba_rom(&mut rom);
        // inject FLASH string for save detection
        rom[0x500..0x509].copy_from_slice(b"FLASH_V13");
        let id = GbaRomIdentity::from_rom(&rom).unwrap();
        assert_eq!(id.title, "POKEMON EMER");
        assert!(id.fixed_valid);
        assert!(id.logo_valid);
        assert_eq!(id.save_type, SaveTypeSer::Flash64);
    }
}
