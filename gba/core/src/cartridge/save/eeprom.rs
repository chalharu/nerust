use super::helpers::{read_slice, write_slice};
use super::{SaveBackend, SaveType};

const EEPROM_SIZE: usize = 8192; // 8KB max, covers 512B as subset

#[derive(Debug)]
pub struct EepromSave {
    data: Vec<u8>,
}

impl EepromSave {
    pub fn new() -> Self {
        Self {
            data: vec![0xFF; EEPROM_SIZE],
        }
    }
}

impl Default for EepromSave {
    fn default() -> Self {
        Self::new()
    }
}

impl SaveBackend for EepromSave {
    fn save_type(&self) -> SaveType {
        // 常時8KB確保のため Eeprom8k を返す
        SaveType::Eeprom8k
    }

    fn read(&self, addr: u32, width: u8) -> u32 {
        // GBA EEPROMは 0x0D000000領域でDMA経由のシリアルアクセス。
        // Phase 4では永続化層を検証するための線形ストレージモデルとする。
        // TODO(gba-eeprom-phase8.5): DMA実装時にシリアルプロトコルへ置換する。
        let off = (addr & 0x1FFF) as usize;
        read_slice(&self.data, off, width)
    }

    fn write(&mut self, addr: u32, width: u8, value: u32) {
        let off = (addr & 0x1FFF) as usize;
        write_slice(&mut self.data, off, width, value);
    }

    fn ram_data(&self) -> Option<&[u8]> {
        Some(&self.data)
    }

    fn ram_restore(&mut self, data: &[u8]) {
        let len = data.len().min(EEPROM_SIZE);
        self.data[..len].copy_from_slice(&data[..len]);
    }

    fn serialize_state(&self) -> Vec<u8> {
        self.data.clone()
    }

    fn deserialize_state(&mut self, data: &[u8]) -> Result<(), String> {
        if data.len() != EEPROM_SIZE {
            return Err(format!("EEPROM state size mismatch: {}", data.len()));
        }
        self.data.copy_from_slice(data);
        Ok(())
    }
}
