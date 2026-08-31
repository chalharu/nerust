use super::helpers::{read_slice, write_slice};
use super::{SaveBackend, SaveType};

const SRAM_SIZE: usize = 0x8000; // 32KB, mirrored to 64KB

#[derive(Debug)]
pub struct SramSave {
    data: Box<[u8; SRAM_SIZE]>,
}

impl SramSave {
    pub fn new() -> Self {
        Self {
            data: Box::new([0xFF; SRAM_SIZE]),
        }
    }
}

impl Default for SramSave {
    fn default() -> Self {
        Self::new()
    }
}

impl SaveBackend for SramSave {
    fn save_type(&self) -> SaveType {
        SaveType::Sram
    }

    fn read(&self, addr: u32, width: u8) -> u32 {
        let off = (addr & 0x7FFF) as usize;
        read_slice(&*self.data, off, width)
    }

    fn write(&mut self, addr: u32, width: u8, value: u32) {
        let off = (addr & 0x7FFF) as usize;
        write_slice(&mut *self.data, off, width, value);
    }

    fn ram_data(&self) -> Option<&[u8]> {
        Some(&self.data[..])
    }

    fn ram_restore(&mut self, data: &[u8]) {
        let len = data.len().min(SRAM_SIZE);
        self.data[..len].copy_from_slice(&data[..len]);
    }

    fn serialize_state(&self) -> Vec<u8> {
        self.data.to_vec()
    }

    fn deserialize_state(&mut self, data: &[u8]) -> Result<(), String> {
        if data.len() != SRAM_SIZE {
            return Err(format!("SRAM state size mismatch: {}", data.len()));
        }
        self.data.copy_from_slice(data);
        Ok(())
    }
}
