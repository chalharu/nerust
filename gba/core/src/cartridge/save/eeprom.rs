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

impl EepromSave {
    pub fn dma_eeprom_write(&mut self, src: &[u16], is_8k: bool) {
        // GBA EEPROM is serial via DMA3: each halfword's LSB is one bit.
        // For HLE we support both packed 16-bit and LSB-serial forms.
        // Try LSB-serial first (90 bits for 8K, 82 for 512)
        let addr_bits = if is_8k { 14 } else { 6 };
        let total_bits = 2 + 2 + addr_bits + 64 + 1;
        // Collect LSBs if src looks like bit-serial (many entries, each 0/1)
        let is_bit_serial = src.len() >= total_bits && src.iter().all(|&w| w <= 1);
        if is_bit_serial {
            // Find header 1,0,1,0
            let mut off = 0;
            while off < src.len() && src[off] == 0 {
                off += 1;
            }
            if off + total_bits <= src.len() && src[off] == 1 && src[off + 1] == 0 && src[off + 2] == 1 && src[off + 3] == 0 {
                let mut addr = 0usize;
                for i in 0..addr_bits {
                    addr = (addr << 1) | (src[off + 4 + i] as usize & 1);
                }
                let mut data = [0u8; 8];
                for i in 0..64 {
                    let bit = (src[off + 4 + addr_bits + i] & 1) as u8;
                    let byte = i / 8;
                    let bit_in_byte = 7 - (i % 8);
                    if bit == 1 {
                        data[byte] |= 1 << bit_in_byte;
                    }
                }
                let eeprom_off = addr * 8;
                if eeprom_off + 8 <= self.data.len() {
                    self.data[eeprom_off..eeprom_off + 8].copy_from_slice(&data);
                }
                return;
            }
        }
        // Fallback: packed 16-bit form (9 halfwords for 8K)
        // src[0] contains address after 4-bit header, src[1..] contains 8 bytes as 4x16-bit LE
        if src.len() >= 5 {
            let addr_mask = (1usize << addr_bits) - 1;
            let addr = ((src[0] as usize) >> 2) & addr_mask;
            let eeprom_off = addr * 8;
            for i in 0..8 {
                if 1 + i / 2 < src.len() && eeprom_off + i < self.data.len() {
                    let w = src[1 + i / 2];
                    let b = if i % 2 == 0 {
                        (w & 0xFF) as u8
                    } else {
                        (w >> 8) as u8
                    };
                    self.data[eeprom_off + i] = b;
                }
            }
        }
    }

    pub fn dma_eeprom_read(&self, dst: &mut [u16], addr: u16, _is_8k: bool) {
        let eeprom_off = (addr as usize) * 8;
        let mut data = [0xFFu8; 8];
        if eeprom_off + 8 <= self.data.len() {
            data.copy_from_slice(&self.data[eeprom_off..eeprom_off + 8]);
        }
        // Encode as LSB-serial if dst is bit-serial sized (>=68), otherwise as packed bytes
        if dst.len() >= 68 {
            // Fill with header 0,0,1,1 + 64 data bits as LSBs
            for v in dst.iter_mut() {
                *v = 0;
            }
            if dst.len() > 4 {
                dst[2] = 1;
                dst[3] = 1;
                for i in 0..64 {
                    let byte = i / 8;
                    let bit_in_byte = 7 - (i % 8);
                    let bit = (data[byte] >> bit_in_byte) & 1;
                    if 4 + i < dst.len() {
                        dst[4 + i] = bit as u16;
                    }
                }
            }
        } else {
            // Packed: 4 halfwords = 8 bytes LE
            for i in 0..4.min(dst.len()) {
                let lo = data[i * 2] as u16;
                let hi = data[i * 2 + 1] as u16;
                dst[i] = lo | (hi << 8);
            }
        }
    }
}

impl SaveBackend for EepromSave {
    fn save_type(&self) -> SaveType {
        SaveType::Eeprom8k
    }

    fn read(&self, addr: u32, width: u8) -> u32 {
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
