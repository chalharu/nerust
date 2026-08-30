pub mod eeprom;
pub mod flash;
pub mod none;
pub mod sram;

use self::eeprom::EepromSave;
use self::flash::FlashSave;
use self::none::NoneSave;
use self::sram::SramSave;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SaveType {
    None,
    Eeprom512,
    Eeprom8k,
    Sram,
    Flash64,
    Flash128,
}

pub trait SaveBackend: std::fmt::Debug + Send {
    fn save_type(&self) -> SaveType;
    fn read(&self, addr: u32, width: u8) -> u32;
    fn write(&mut self, addr: u32, width: u8, value: u32);
    fn has_battery(&self) -> bool {
        !matches!(self.save_type(), SaveType::None)
    }
    fn ram_data(&self) -> Option<&[u8]>;
    fn ram_restore(&mut self, data: &[u8]);
    fn serialize_state(&self) -> Vec<u8>;
    fn deserialize_state(&mut self, data: &[u8]) -> Result<(), String>;
}

pub fn detect_save_type(rom: &[u8]) -> SaveType {
    // GBAヘッダにセーブ情報がないため SDK文字列を word-aligned step_by(4) でスキャン
    // 優先順: FLASH1M > FLASH512/FLASH > SRAM > EEPROM > None
    let mut found_sram = false;
    let mut found_eeprom = false;
    let mut found_flash = false;
    let mut found_flash1m = false;

    // Efficient scan: check every 4 bytes for known strings
    for i in (0..rom.len()).step_by(4) {
        let slice = &rom[i..];
        if slice.starts_with(b"FLASH1M_V") {
            found_flash1m = true;
            break;
        }
        if slice.starts_with(b"FLASH512_V") || slice.starts_with(b"FLASH_V") {
            found_flash = true;
        } else if slice.starts_with(b"SRAM_V") || slice.starts_with(b"SRAM_F_V") {
            found_sram = true;
        } else if slice.starts_with(b"EEPROM_V") {
            found_eeprom = true;
        }
    }

    if found_flash1m {
        return SaveType::Flash128;
    }
    if found_flash {
        return SaveType::Flash64;
    }
    if found_sram {
        return SaveType::Sram;
    }
    if found_eeprom {
        // EEPROM_Vだけでは 512B/8KB 区別不可。常時8KBで確保するため Eeprom8k を返す。
        return SaveType::Eeprom8k;
    }
    SaveType::None
}

pub fn create_save_backend(save_type: SaveType) -> Box<dyn SaveBackend> {
    match save_type {
        SaveType::None => Box::new(NoneSave::new()),
        SaveType::Sram => Box::new(SramSave::new()),
        SaveType::Eeprom512 | SaveType::Eeprom8k => Box::new(EepromSave::new()),
        SaveType::Flash64 => Box::new(FlashSave::new(false)),
        SaveType::Flash128 => Box::new(FlashSave::new(true)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detect_none() {
        let rom = vec![0u8; 0x1000];
        assert_eq!(detect_save_type(&rom), SaveType::None);
    }

    #[test]
    fn detect_sram() {
        let mut rom = vec![0u8; 0x1000];
        rom[0x100..0x107].copy_from_slice(b"SRAM_V1");
        assert_eq!(detect_save_type(&rom), SaveType::Sram);
    }

    #[test]
    fn detect_flash128_priority() {
        let mut rom = vec![0u8; 0x1000];
        rom[0x100..0x10A].copy_from_slice(b"EEPROM_V12");
        rom[0x200..0x209].copy_from_slice(b"FLASH1M_V");
        assert_eq!(detect_save_type(&rom), SaveType::Flash128);
    }

    #[test]
    fn detect_eeprom() {
        let mut rom = vec![0u8; 0x1000];
        rom[0x100..0x109].copy_from_slice(b"EEPROM_V1");
        assert_eq!(detect_save_type(&rom), SaveType::Eeprom8k);
    }

    #[test]
    fn detect_flash64() {
        let mut rom = vec![0u8; 0x1000];
        rom[0x100..0x108].copy_from_slice(b"FLASH_V1");
        assert_eq!(detect_save_type(&rom), SaveType::Flash64);
    }
}
