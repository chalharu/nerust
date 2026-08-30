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

fn read_slice(slice: &[u8], off: usize, width: u8) -> u32 {
    match width {
        4 => {
            let b0 = *slice.get(off).unwrap_or(&0xFF) as u32;
            let b1 = *slice.get(off + 1).unwrap_or(&0xFF) as u32;
            let b2 = *slice.get(off + 2).unwrap_or(&0xFF) as u32;
            let b3 = *slice.get(off + 3).unwrap_or(&0xFF) as u32;
            b0 | (b1 << 8) | (b2 << 16) | (b3 << 24)
        }
        2 => {
            let b0 = *slice.get(off).unwrap_or(&0xFF) as u32;
            let b1 = *slice.get(off + 1).unwrap_or(&0xFF) as u32;
            b0 | (b1 << 8)
        }
        _ => *slice.get(off).unwrap_or(&0xFF) as u32,
    }
}

fn write_slice(slice: &mut [u8], off: usize, width: u8, value: u32) {
    match width {
        4 => {
            if let Some(b) = slice.get_mut(off) {
                *b = (value & 0xFF) as u8;
            }
            if let Some(b) = slice.get_mut(off + 1) {
                *b = ((value >> 8) & 0xFF) as u8;
            }
            if let Some(b) = slice.get_mut(off + 2) {
                *b = ((value >> 16) & 0xFF) as u8;
            }
            if let Some(b) = slice.get_mut(off + 3) {
                *b = ((value >> 24) & 0xFF) as u8;
            }
        }
        2 => {
            if let Some(b) = slice.get_mut(off) {
                *b = (value & 0xFF) as u8;
            }
            if let Some(b) = slice.get_mut(off + 1) {
                *b = ((value >> 8) & 0xFF) as u8;
            }
        }
        _ => {
            if let Some(b) = slice.get_mut(off) {
                *b = (value & 0xFF) as u8;
            }
        }
    }
}
