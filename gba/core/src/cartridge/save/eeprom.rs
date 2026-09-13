use super::helpers::{read_slice, write_slice};
use super::{SaveBackend, SaveType};

const EEPROM_SIZE: usize = 8192; // 8KB max, covers 512B as subset

/// EEPROM serial state machine (GBATEK Backup Media / EEPROM).
///
/// The chip is bit-serial and DMA-only: each 16-bit DMA unit carries one bit
/// in bit 0. Write frame: `1` start, `0` op, 6/14 address bits (MSB first),
/// 64 data bits, `0` stop. Read request: `1` start, `1` op, address bits;
/// the game then DMA-reads 68 units (4 dummy + 64 data bits) from 0D000000h.
///
/// 512B vs 8KB is latched from the first decodable frame (mGBA-style):
/// an exact 73-bit write / 8-bit read request means 512B, 81/16 means 8KB.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EepromAddrWidth {
    Bits512,
    Bits8k,
}

#[derive(Debug)]
pub struct EepromSave {
    data: Vec<u8>,
    frame: Vec<bool>,
    size_8k: Option<bool>,
    read_queue: Vec<bool>,
}

impl EepromSave {
    pub fn new() -> Self {
        Self {
            data: vec![0xFF; EEPROM_SIZE],
            frame: Vec::new(),
            size_8k: None,
            read_queue: Vec::new(),
        }
    }

    fn addr_bits(&self) -> u8 {
        if self.size_8k == Some(false) { 6 } else { 14 }
    }

    fn commit_read_request(&mut self, addr: usize) {
        self.read_queue.clear();
        // 4 dummy bits (ignored by games) + 64 data bits, MSB first.
        // GBATEK: 8KB EEPROMs use only the lower 10 address bits (upper 4
        // must be zero); mask so aliased addresses read the same block.
        let addr = if self.size_8k == Some(true) {
            addr & 0x3FF
        } else {
            addr
        };
        self.read_queue.extend_from_slice(&[false; 4]);
        let base = addr * 8;
        for i in 0..64 {
            let byte = self.data.get(base + i / 8).copied().unwrap_or(0xFF);
            self.read_queue.push((byte >> (7 - (i % 8)) & 1) != 0);
        }
    }

    /// Expose the latched address width for tests.
    pub fn latched_width(&self) -> Option<EepromAddrWidth> {
        self.size_8k.map(|b| {
            if b {
                EepromAddrWidth::Bits8k
            } else {
                EepromAddrWidth::Bits512
            }
        })
    }
}

impl Default for EepromSave {
    fn default() -> Self {
        Self::new()
    }
}

impl EepromSave {
    /// Feed one serial bit (halfword bit 0) of a DMA write burst to 0D000000h.
    /// Bits are buffered; the frame is decoded when the burst ends, since
    /// 512B vs 8KB address width is only known then (first-frame latch).
    pub fn serial_write_bit(&mut self, bit: bool) {
        self.frame.push(bit);
    }

    /// End of a DMA burst: decode the buffered frame, latch 512B/8KB from
    /// the first decodable frame, and commit it.
    pub fn end_burst(&mut self) {
        let frame = std::mem::take(&mut self.frame);
        if frame.len() < 8 || !frame[0] {
            return;
        }
        let is_read = frame[1];
        let try_width = |width: usize| -> Option<(usize, usize)> {
            // Returns (addr, data_end) for a valid frame of this width.
            let addr_bits = if width == 8 { 6 } else { 14 };
            if frame.len() < width {
                return None;
            }
            if is_read {
                // GBATEK EEPROM read request: `11` + addr(6/14) + `0`
                // (9/17 bits). Accept the trailing stop bit or its omission
                // (both seen in the wild), but reject overlong frames.
                let exact = 2 + addr_bits;
                let with_stop = exact + 1;
                if frame.len() != exact && frame.len() != with_stop {
                    return None;
                }
                if frame.len() == with_stop && frame[exact] {
                    return None;
                }
                let mut addr = 0usize;
                for i in 0..addr_bits {
                    addr = (addr << 1) | usize::from(frame[2 + i]);
                }
                Some((addr, width))
            } else {
                // GBATEK EEPROM write frame: `10` + addr(6/14) + 64 data
                // bits + `0` stop (73/81 bits exactly). Enforce the exact
                // length: trailing garbage is rejected, and short frames
                // return None instead of panicking on frame[stop] below.
                let stop = 2 + addr_bits + 64;
                if frame.len() != stop + 1 || frame[stop] {
                    return None;
                }
                let mut addr = 0usize;
                for i in 0..addr_bits {
                    addr = (addr << 1) | usize::from(frame[2 + i]);
                }
                Some((addr, stop + 1))
            }
        };
        // With known size only that width is valid; otherwise an exact
        // 73-bit write / 8-bit read request means 512B (short frames can
        // never be 8KB), longer valid frames mean 8KB.
        let order_512_first = match self.size_8k {
            Some(false) => true,
            Some(true) => false,
            None => frame.len() < 77,
        };
        let widths: [usize; 2] = if order_512_first { [8, 16] } else { [16, 8] };
        for width in widths {
            if self.size_8k.is_some() && width != (if self.size_8k == Some(false) { 8 } else { 16 })
            {
                continue;
            }
            if let Some((addr, _)) = try_width(width) {
                self.size_8k = Some(width == 16);
                if is_read {
                    self.commit_read_request(addr);
                } else {
                    let addr_bits = if width == 8 { 6 } else { 14 };
                    let mut data = [0u8; 8];
                    for i in 0..64 {
                        if frame[2 + addr_bits + i] {
                            data[i / 8] |= 1 << (7 - (i % 8));
                        }
                    }
                    // 10-bit alias for 8KB (see commit_read_request).
                    let addr = if width == 16 { addr & 0x3FF } else { addr };
                    let base = addr * 8;
                    if base + 8 <= self.data.len() {
                        self.data[base..base + 8].copy_from_slice(&data);
                    }
                }
                return;
            }
        }
    }

    /// Pop the next response bit for a DMA read from 0D000000h.
    /// Outside a committed read request the chip drives 1 (pulled up).
    pub fn serial_read_bit(&mut self) -> bool {
        if self.read_queue.is_empty() {
            return true;
        }
        self.read_queue.remove(0)
    }

    /// Peek the response level without consuming (for CPU loads).
    pub fn serial_peek_bit(&self) -> bool {
        self.read_queue.first().copied().unwrap_or(true)
    }

    /// Number of address bits currently assumed (latched or 14-bit default).
    pub fn assumed_addr_bits(&self) -> u8 {
        self.addr_bits()
    }
}

impl SaveBackend for EepromSave {
    fn save_type(&self) -> SaveType {
        if self.size_8k == Some(false) {
            SaveType::Eeprom512
        } else {
            SaveType::Eeprom8k
        }
    }

    fn read(&self, addr: u32, width: u8) -> u32 {
        // Parallel CPU access is not possible on hardware (DMA-only serial
        // chip); kept as a direct window for tooling/tests.
        let off = (addr & 0x1FFF) as usize;
        read_slice(&self.data, off, width)
    }

    fn eeprom_write_bit(&mut self, bit: bool) {
        self.serial_write_bit(bit);
    }

    fn eeprom_read_bit(&mut self) -> bool {
        self.serial_read_bit()
    }

    fn eeprom_peek_bit(&self) -> bool {
        self.serial_peek_bit()
    }

    fn eeprom_end_burst(&mut self) {
        self.end_burst();
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
        let mut out = Vec::with_capacity(self.data.len() + 1);
        out.extend_from_slice(&self.data);
        out.push(match self.size_8k {
            Some(false) => 0,
            Some(true) => 1,
            None => 0xFF,
        });
        out
    }

    fn deserialize_state(&mut self, data: &[u8]) -> Result<(), String> {
        if data.len() != EEPROM_SIZE + 1 {
            return Err(format!("EEPROM state size mismatch: {}", data.len()));
        }
        self.data.copy_from_slice(&data[..EEPROM_SIZE]);
        self.size_8k = match data[EEPROM_SIZE] {
            0 => Some(false),
            1 => Some(true),
            _ => None,
        };
        self.frame.clear();
        self.read_queue.clear();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_frame(addr_bits: u8, addr: u16, data: &[u8; 8]) -> Vec<bool> {
        let mut bits = vec![true, false];
        for i in (0..addr_bits).rev() {
            bits.push((addr >> i) & 1 != 0);
        }
        for byte in data {
            for i in (0..8).rev() {
                bits.push((byte >> i) & 1 != 0);
            }
        }
        bits.push(false); // stop
        bits
    }

    #[test]
    fn serial_write_8k_roundtrip() {
        let mut eeprom = EepromSave::new();
        let data = [0x12, 0x34, 0x56, 0x78, 0x9A, 0xBC, 0xDE, 0xF0];
        for bit in write_frame(14, 5, &data) {
            eeprom.serial_write_bit(bit);
        }
        eeprom.end_burst();
        assert_eq!(eeprom.latched_width(), Some(EepromAddrWidth::Bits8k));
        assert_eq!(eeprom.save_type(), SaveType::Eeprom8k);
        assert_eq!(&eeprom.data[40..48], &data);
        // Read request: start, read-op, 14-bit addr.
        let mut req = vec![true, true];
        for i in (0..14).rev() {
            req.push((5 >> i) & 1 != 0);
        }
        for bit in req {
            eeprom.serial_write_bit(bit);
        }
        eeprom.end_burst();
        let mut out = Vec::new();
        for _ in 0..68 {
            out.push(eeprom.serial_read_bit());
        }
        let mut got = [0u8; 8];
        for (i, bit) in out.iter().skip(4).enumerate() {
            if *bit {
                got[i / 8] |= 1 << (7 - (i % 8));
            }
        }
        assert_eq!(got, data);
    }

    #[test]
    fn serial_write_512b_latches_short_size() {
        let mut eeprom = EepromSave::new();
        let data = [0xAA; 8];
        for bit in write_frame(6, 3, &data) {
            eeprom.serial_write_bit(bit);
        }
        eeprom.end_burst();
        assert_eq!(eeprom.latched_width(), Some(EepromAddrWidth::Bits512));
        assert_eq!(eeprom.save_type(), SaveType::Eeprom512);
        assert_eq!(&eeprom.data[24..32], &data);
    }
}
