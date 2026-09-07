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
            if off + total_bits <= src.len()
                && src[off] == 1
                && src[off + 1] == 0
                && src[off + 2] == 1
                && src[off + 3] == 0
            {
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
        // Fallback: packed 16-bit form (9 halfwords = 144 bits for 8K, 90 bits needed)
        // Collect 16 bits per halfword MSB first and search for header.
        let mut packed_bits = Vec::with_capacity(src.len() * 16);
        for &w in src {
            for b in (0..16).rev() {
                packed_bits.push((w >> b) & 1);
            }
        }
        // Find header 1,0,1,0 in packed stream
        for off in 0..packed_bits.len().saturating_sub(total_bits) {
            if packed_bits[off] == 1
                && packed_bits[off + 1] == 0
                && packed_bits[off + 2] == 1
                && packed_bits[off + 3] == 0
            {
                let mut addr = 0usize;
                for i in 0..addr_bits {
                    addr = (addr << 1) | (packed_bits[off + 4 + i] as usize);
                }
                let mut data = [0u8; 8];
                for i in 0..64 {
                    let bit = packed_bits[off + 4 + addr_bits + i] as u8;
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
    }

    pub fn dma_eeprom_read(&self, dst: &mut [u16], addr: u16, _is_8k: bool) {
        let eeprom_off = (addr as usize) * 8;
        let mut data = [0xFFu8; 8];
        if eeprom_off + 8 <= self.data.len() {
            data.copy_from_slice(&self.data[eeprom_off..eeprom_off + 8]);
        }
        // GBATEK: EEPROM read via DMA returns 64 data bits after 4 dummy bits.
        // Bit-serial form uses 1 bit per halfword (LSB), packed form uses 16 bits per halfword.
        if dst.len() >= 68 {
            // Bit-serial: 68 halfwords = 4 dummy + 64 data (LSB per halfword)
            for v in dst.iter_mut() {
                *v = 0;
            }
            // Header for read is 0,0,1,1 per some docs, but data starts at +4
            if dst.len() > 4 {
                // Some docs show header 1,1 for read
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
            // For 73 halfwords (8K read), the extra 5 are dummy 0
        } else if dst.len() == 73 || dst.len() == 9 {
            // Packed 16-bit form: fill with header + data packed MSB first
            // Clear first
            for v in dst.iter_mut() {
                *v = 0;
            }
            // Build bitstream: 4 dummy + 64 data
            let mut bits = Vec::with_capacity(68);
            bits.extend_from_slice(&[0, 0, 1, 1]);
            for i in 0..64 {
                let byte = i / 8;
                let bit_in_byte = 7 - (i % 8);
                bits.push(((data[byte] >> bit_in_byte) & 1) as u16);
            }
            // Pack bits MSB first into dst's 16-bit words
            for (i, chunk) in bits.chunks(16).enumerate() {
                if i >= dst.len() {
                    break;
                }
                let mut w = 0u16;
                for &b in chunk {
                    w = (w << 1) | b;
                }
                // Pad remaining bits in chunk with 0
                w <<= 16 - chunk.len();
                dst[i] = w;
            }
        } else {
            // Packed: 4 halfwords = 8 bytes LE (simple case for tests)
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
