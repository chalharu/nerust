use super::helpers::read_slice;
use super::{SaveBackend, SaveType};

const EEPROM_SIZE: usize = 8192; // 8KB max, covers 512B as subset

/// Pack a bit vec as u16-LE count + MSB-first bytes (zero padding).
fn pack_bits(out: &mut Vec<u8>, bits: &[bool]) {
    let count = bits.len().min(0xFFFF);
    out.extend_from_slice(&(count as u16).to_le_bytes());
    let mut byte = 0u8;
    for (i, bit) in bits.iter().take(count).enumerate() {
        byte |= u8::from(*bit) << (7 - (i % 8));
        if i % 8 == 7 {
            out.push(byte);
            byte = 0;
        }
    }
    if !count.is_multiple_of(8) {
        out.push(byte);
    }
}

/// Inverse of [`pack_bits`]: returns the bits and the next cursor.
/// `max_bits` bounds legitimate lengths (frame bursts vs 68-bit responses).
fn unpack_bits(data: &[u8], cursor: usize, max_bits: usize) -> Result<(Vec<bool>, usize), String> {
    if data.len() < cursor + 2 {
        return Err(format!("EEPROM state truncated at {cursor}"));
    }
    let count = u16::from_le_bytes([data[cursor], data[cursor + 1]]) as usize;
    if count > max_bits {
        return Err(format!("EEPROM bit run too long: {count}"));
    }
    let bytes = count.div_ceil(8);
    if data.len() < cursor + 2 + bytes {
        return Err("EEPROM state truncated in bit run".to_string());
    }
    let mut bits = Vec::with_capacity(count);
    for i in 0..count {
        bits.push(data[cursor + 2 + i / 8] >> (7 - (i % 8)) & 1 != 0);
    }
    // Padding bits must be zero (the packer always writes zero).
    let pad = bytes * 8 - count;
    if pad > 0 {
        let last = data[cursor + 2 + bytes - 1];
        if last & ((1 << pad) - 1) != 0 {
            return Err("EEPROM state has nonzero padding bits".to_string());
        }
    }
    Ok((bits, cursor + 2 + bytes))
}

/// EEPROM bit-serial DMA-only state machine.
/// Write frames carry start/op/address/data/stop; reads return 4 dummy + 64 data bits.
/// 512B vs 8KB address width is latched from the first decodable frame.
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
        for width in self.probe_order(frame.len()) {
            if self.width_ruled_out(width) {
                continue;
            }
            let addr_bits = if width == 8 { 6 } else { 14 };
            let addr = if is_read {
                decode_read_addr(&frame, addr_bits)
            } else {
                decode_write_addr(&frame, addr_bits)
            };
            if let Some(addr) = addr {
                self.size_8k = Some(width == 16);
                if is_read {
                    self.commit_read_request(addr);
                } else {
                    self.commit_write_data(&frame, width, addr_bits, addr);
                }
                return;
            }
        }
    }

    /// Width probe order: with known size only that width is valid;
    /// otherwise an exact 73-bit write / 8-bit read request means 512B
    /// (short frames can never be 8KB), longer valid frames mean 8KB.
    fn probe_order(&self, frame_len: usize) -> [usize; 2] {
        let order_512_first = match self.size_8k {
            Some(false) => true,
            Some(true) => false,
            None => frame_len < 77,
        };
        if order_512_first { [8, 16] } else { [16, 8] }
    }

    /// True when a latched size rules this probe width out.
    fn width_ruled_out(&self, width: usize) -> bool {
        match self.size_8k {
            Some(large) => width != if large { 16 } else { 8 },
            None => false,
        }
    }

    /// Commit a decoded write frame's 64 data bits.
    fn commit_write_data(&mut self, frame: &[bool], width: usize, addr_bits: usize, addr: usize) {
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

/// GBATEK EEPROM read request: `11` + addr(6/14) + `0` (9/17 bits).
/// Accept the trailing stop bit or its omission (both seen in the
/// wild), but reject overlong frames.
fn decode_read_addr(frame: &[bool], addr_bits: usize) -> Option<usize> {
    let exact = 2 + addr_bits;
    let with_stop = exact + 1;
    if frame.len() != exact && frame.len() != with_stop {
        return None;
    }
    if frame.len() == with_stop && frame[exact] {
        return None;
    }
    Some(read_addr_bits(frame, addr_bits))
}

/// GBATEK EEPROM write frame: `10` + addr(6/14) + 64 data bits + `0`
/// stop (73/81 bits exactly). Enforce the exact length: trailing
/// garbage is rejected, and short frames return None instead of
/// panicking on frame[stop].
fn decode_write_addr(frame: &[bool], addr_bits: usize) -> Option<usize> {
    let stop = 2 + addr_bits + 64;
    if frame.len() != stop + 1 || frame[stop] {
        return None;
    }
    Some(read_addr_bits(frame, addr_bits))
}

fn read_addr_bits(frame: &[bool], addr_bits: usize) -> usize {
    let mut addr = 0usize;
    for i in 0..addr_bits {
        addr = (addr << 1) | usize::from(frame[2 + i]);
    }
    addr
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

    fn write(&mut self, _addr: u32, _width: u8, _value: u32) {
        // GBATEK: manual LDRH/STRH transfers won't work — the chip is
        // DMA3-serial only. Direct stores are ignored, never bypassing
        // the protocol.
        debug_assert!(false, "direct EEPROM write ignored (DMA-only chip)");
    }

    fn ram_data(&self) -> Option<&[u8]> {
        Some(&self.data)
    }

    fn ram_restore(&mut self, data: &[u8]) {
        let len = data.len().min(EEPROM_SIZE);
        self.data[..len].copy_from_slice(&data[..len]);
    }

    fn serialize_state(&self) -> Vec<u8> {
        // data + size flag + in-flight serial state (a save may land
        // mid-burst: the open frame bits and the pending read response
        // must survive for an exact resume).
        let mut out = Vec::with_capacity(self.data.len() + 1 + 4 + 32);
        out.extend_from_slice(&self.data);
        out.push(match self.size_8k {
            Some(false) => 0,
            Some(true) => 1,
            None => 0xFF,
        });
        pack_bits(&mut out, &self.frame);
        pack_bits(&mut out, &self.read_queue);
        out
    }

    fn deserialize_state(&mut self, data: &[u8]) -> Result<(), String> {
        let mut cursor = EEPROM_SIZE + 1;
        if data.len() < cursor {
            return Err(format!("EEPROM state size mismatch: {}", data.len()));
        }
        if data.len() == cursor {
            // Legacy blob (Phase 10 pre-bitstream): data + size flag only.
            self.data.copy_from_slice(&data[..EEPROM_SIZE]);
            self.size_8k = match data[EEPROM_SIZE] {
                0 => Some(false),
                1 => Some(true),
                _ => None,
            };
            self.frame.clear();
            self.read_queue.clear();
            return Ok(());
        }
        let (frame, next) = unpack_bits(data, cursor, 0x1_0000)?;
        cursor = next;
        let (read_queue, next) = unpack_bits(data, cursor, 68)?;
        cursor = next;
        if cursor != data.len() {
            return Err(format!(
                "EEPROM state trailing bytes: {}",
                data.len() - cursor
            ));
        }
        self.data.copy_from_slice(&data[..EEPROM_SIZE]);
        self.size_8k = match data[EEPROM_SIZE] {
            0 => Some(false),
            1 => Some(true),
            _ => None,
        };
        self.frame = frame;
        self.read_queue = read_queue;
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

    #[test]
    fn state_round_trips_mid_frame_and_mid_response() {
        let data = [0x12, 0x34, 0x56, 0x78, 0x9A, 0xBC, 0xDE, 0xF0];
        let frame = write_frame(14, 5, &data);
        // Reference: uninterrupted write + read request.
        let mut reference = EepromSave::new();
        for bit in &frame {
            reference.serial_write_bit(*bit);
        }
        reference.end_burst();
        // Save the writer mid-frame (after the address, before data end).
        let mut writer = EepromSave::new();
        for bit in &frame[..20] {
            writer.serial_write_bit(*bit);
        }
        let blob = writer.serialize_state();
        let mut restored = EepromSave::new();
        restored.deserialize_state(&blob).unwrap();
        for bit in &frame[20..] {
            restored.serial_write_bit(*bit);
        }
        restored.end_burst();
        assert_eq!(restored.data, reference.data);
        assert_eq!(restored.latched_width(), reference.latched_width());
        // Commit a read request on both, then save mid-response.
        let mut req = vec![true, true];
        for i in (0..14).rev() {
            req.push((5 >> i) & 1 != 0);
        }
        for bit in &req {
            reference.serial_write_bit(*bit);
            restored.serial_write_bit(*bit);
        }
        reference.end_burst();
        restored.end_burst();
        let mut ref_bits = Vec::new();
        let mut got_bits = Vec::new();
        for _ in 0..30 {
            ref_bits.push(reference.serial_read_bit());
            got_bits.push(restored.serial_read_bit());
        }
        let blob = restored.serialize_state();
        let mut restored2 = EepromSave::new();
        restored2.deserialize_state(&blob).unwrap();
        for _ in 0..38 {
            ref_bits.push(reference.serial_read_bit());
            got_bits.push(restored2.serial_read_bit());
        }
        assert_eq!(got_bits, ref_bits);
    }

    #[test]
    fn state_rejects_garbage() {
        let mut eeprom = EepromSave::new();
        assert!(eeprom.deserialize_state(&[0u8; 10]).is_err());
        let mut blob = eeprom.serialize_state();
        // Overlong frame count.
        let frame_at = EEPROM_SIZE + 1;
        blob[frame_at] = 0xFF;
        blob[frame_at + 1] = 0xFF;
        assert!(eeprom.deserialize_state(&blob).is_err());
        // Trailing bytes.
        let mut blob = eeprom.serialize_state();
        blob.push(0);
        assert!(eeprom.deserialize_state(&blob).is_err());
    }
}
