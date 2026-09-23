use super::helpers::{repeat_byte, selected_write_byte};
use super::{SaveBackend, SaveType};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FlashState {
    Ready,
    Unlock1, // after AA at 5555
    Unlock2, // after 55 at 2AAA
}

#[derive(Debug)]
pub struct FlashSave {
    data: Vec<u8>,
    is_128k: bool,
    bank: usize, // 0 or 1 for 128K — 現在アクティブな64KBバンク
    state: FlashState,
    id_mode: bool,
    bank_switch_pending: bool, // 0xB0 コマンド後の次回 0x0E000000 書き込み待ち
    program_pending: bool,     // 0xA0 コマンド後の次回書き込みでデータ反映
}

impl FlashSave {
    pub fn new(is_128k: bool) -> Self {
        let size = if is_128k { 0x20000 } else { 0x10000 };
        Self {
            data: vec![0xFF; size],
            is_128k,
            bank: 0,
            state: FlashState::Ready,
            id_mode: false,
            bank_switch_pending: false,
            program_pending: false,
        }
    }

    fn bank_offset(&self) -> usize {
        if self.is_128k { self.bank * 0x10000 } else { 0 }
    }
}

impl SaveBackend for FlashSave {
    fn save_type(&self) -> SaveType {
        if self.is_128k {
            SaveType::Flash128
        } else {
            SaveType::Flash64
        }
    }

    fn read(&self, addr: u32, width: u8) -> u32 {
        if self.id_mode {
            let off = (addr & 1) as usize;
            let manufacturer = 0x32; // Panasonic
            let device = if self.is_128k { 0x13 } else { 0x1B };
            let val = if off == 0 { manufacturer } else { device };
            return match width {
                4 => val as u32 | ((val as u32) << 8) | ((val as u32) << 16) | ((val as u32) << 24),
                2 => val as u32 | ((val as u32) << 8),
                _ => val as u32,
            };
        }
        let off = ((addr & 0xFFFF) as usize) + self.bank_offset();
        repeat_byte(self.data[off], width)
    }

    fn write(&mut self, addr: u32, width: u8, value: u32) {
        let low = addr & 0xFFFF;
        let byte = selected_write_byte(addr, width, value);
        // A0 program data and B0 bank selection take priority over a new unlock sequence.
        if self.finish_program(addr, byte) || self.finish_bank_switch(addr, byte) {
            return;
        }
        match self.state {
            FlashState::Ready => self.accept_unlock(low, byte),
            FlashState::Unlock1 => self.accept_unlock2(low, byte),
            FlashState::Unlock2 => self.execute_command(addr, byte),
        }
    }

    fn ram_data(&self) -> Option<&[u8]> {
        Some(&self.data)
    }

    fn ram_restore(&mut self, data: &[u8]) {
        let len = data.len().min(self.data.len());
        self.data[..len].copy_from_slice(&data[..len]);
    }

    fn serialize_state(&self) -> Vec<u8> {
        self.data.clone()
    }

    fn deserialize_state(&mut self, data: &[u8]) -> Result<(), String> {
        if data.len() != self.data.len() {
            return Err(format!("Flash state size mismatch: {}", data.len()));
        }
        self.data.copy_from_slice(data);
        Ok(())
    }
}

impl FlashSave {
    fn finish_program(&mut self, address: u32, value: u8) -> bool {
        if !self.program_pending {
            return false;
        }
        let offset = (address & 0xFFFF) as usize + self.bank_offset();
        if let Some(byte) = self.data.get_mut(offset) {
            *byte &= value;
        }
        self.program_pending = false;
        true
    }

    fn finish_bank_switch(&mut self, address: u32, value: u8) -> bool {
        if !self.bank_switch_pending {
            return false;
        }
        if self.is_128k && address == 0x0E000000 && value <= 1 {
            self.bank = usize::from(value);
        }
        self.bank_switch_pending = false;
        true
    }

    fn accept_unlock(&mut self, low: u32, value: u8) {
        if low == 0x5555 && value == 0xAA {
            self.state = FlashState::Unlock1;
        }
    }

    fn accept_unlock2(&mut self, low: u32, value: u8) {
        self.state = if low == 0x2AAA && value == 0x55 {
            FlashState::Unlock2
        } else {
            FlashState::Ready
        };
    }

    fn execute_command(&mut self, address: u32, command: u8) {
        // This is reached only after the AA@5555, 55@2AAA unlock sequence.
        self.state = FlashState::Ready;
        match command {
            0x90 => self.id_mode = true,
            0xF0 => self.id_mode = false,
            // 80 is erase setup; the following AA/55 unlock leads to 10 or 30.
            0x80 => {}
            // A0 programs the next byte; B0 selects the next 64 KiB bank.
            0xA0 => self.program_pending = true,
            0xB0 => self.bank_switch_pending = true,
            // 10 erases the chip and 30 erases the addressed 4 KiB sector.
            0x10 => self.data.fill(0xFF),
            0x30 => self.erase_sector(address),
            _ => self.program_byte(address, command),
        }
    }

    fn erase_sector(&mut self, address: u32) {
        let start = ((address & 0xFFFF) as usize & !0xFFF) + self.bank_offset();
        let end = (start + 0x1000).min(self.data.len());
        self.data[start..end].fill(0xFF);
    }

    fn program_byte(&mut self, address: u32, value: u8) {
        let offset = (address & 0xFFFF) as usize + self.bank_offset();
        if let Some(byte) = self.data.get_mut(offset) {
            *byte &= value;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bank_switch_via_b0() {
        let mut flash = FlashSave::new(true); // 128K
        assert_eq!(flash.bank, 0);
        // Write to bank 0 (AA 55 A0 + data)
        flash.write(0x0E005555, 1, 0xAA);
        flash.write(0x0E002AAA, 1, 0x55);
        flash.write(0x0E005555, 1, 0xA0);
        flash.write(0x0E000000, 1, 0x12);
        assert_eq!(flash.read(0x0E000000, 1), 0x12);
        // Switch to bank 1 via B0 sequence
        flash.write(0x0E005555, 1, 0xAA);
        flash.write(0x0E002AAA, 1, 0x55);
        flash.write(0x0E005555, 1, 0xB0);
        flash.write(0x0E000000, 1, 0x01);
        assert_eq!(flash.bank, 1);
        // Write to bank 1
        flash.write(0x0E005555, 1, 0xAA);
        flash.write(0x0E002AAA, 1, 0x55);
        flash.write(0x0E005555, 1, 0xA0);
        flash.write(0x0E000000, 1, 0x34);
        assert_eq!(flash.read(0x0E000000, 1), 0x34);
        // Switch back to bank 0
        flash.write(0x0E005555, 1, 0xAA);
        flash.write(0x0E002AAA, 1, 0x55);
        flash.write(0x0E005555, 1, 0xB0);
        flash.write(0x0E000000, 1, 0x00);
        assert_eq!(flash.bank, 0);
        assert_eq!(flash.read(0x0E000000, 1), 0x12);
        // Direct write without B0 should not switch (Phase 4 strict)
        flash.write(0x0E000000, 1, 0x01);
        assert_eq!(flash.bank, 0); // still 0
    }

    #[test]
    fn sector_erase_clears_4k() {
        let mut flash = FlashSave::new(false);
        // Program a byte
        flash.data[0x1000] = 0x00;
        assert_eq!(flash.data[0x1000], 0x00);
        // Sector erase sequence
        flash.write(0x0E005555, 1, 0xAA);
        flash.write(0x0E002AAA, 1, 0x55);
        flash.write(0x0E005555, 1, 0x80);
        flash.write(0x0E005555, 1, 0xAA);
        flash.write(0x0E002AAA, 1, 0x55);
        flash.write(0x0E001000, 1, 0x30);
        assert_eq!(flash.data[0x1000], 0xFF);
        assert_eq!(flash.data[0x1FFF], 0xFF);
        // Adjacent sector should remain (we didn't erase it, but initially FF)
        flash.data[0x2000] = 0x00;
        flash.write(0x0E005555, 1, 0xAA);
        flash.write(0x0E002AAA, 1, 0x55);
        flash.write(0x0E005555, 1, 0x80);
        flash.write(0x0E005555, 1, 0xAA);
        flash.write(0x0E002AAA, 1, 0x55);
        flash.write(0x0E001000, 1, 0x30);
        assert_eq!(flash.data[0x2000], 0x00); // untouched
    }
}
