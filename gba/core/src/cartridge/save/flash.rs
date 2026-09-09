use super::helpers::{repeat_byte, selected_write_byte};
use super::{SaveBackend, SaveType};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FlashState {
    Ready,
    Unlock1,      // after AA at 5555
    Unlock2,      // after 55 at 2AAA
    ProgramArmed, // after A0: next write is program data
    BankArmed,    // after B0: next write at 0E000000 selects the bank
    EraseSetup,   // after 80: erase command, needs a fresh AA/55 unlock
    EraseUnlock1, // after erase-setup AA at 5555
    EraseUnlock2, // after erase-setup 55 at 2AAA
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FlashChip {
    Panasonic64,
    Sst64,
    Macronix64,
    Atmel64,
    Sanyo128,
    Macronix128,
}

impl FlashChip {
    /// GBATEK FlashROM Device Types (ID = device+maker, MSB first):
    /// D4BFh SST 64K, 1CC2h Macronix 64K, 1B32h Panasonic 64K,
    /// 3D1Fh Atmel 64K, 1362h Sanyo 128K, 09C2h Macronix 128K.
    pub fn manufacturer(self) -> u8 {
        match self {
            FlashChip::Panasonic64 => 0x32,
            FlashChip::Sst64 => 0xBF,
            FlashChip::Macronix64 => 0xC2,
            FlashChip::Atmel64 => 0x1F,
            FlashChip::Sanyo128 => 0x62,
            FlashChip::Macronix128 => 0xC2,
        }
    }

    pub fn device(self) -> u8 {
        match self {
            FlashChip::Panasonic64 => 0x1B,
            FlashChip::Sst64 => 0xD4,
            FlashChip::Macronix64 => 0x1C,
            FlashChip::Atmel64 => 0x3D,
            FlashChip::Sanyo128 => 0x13,
            FlashChip::Macronix128 => 0x09,
        }
    }

    pub fn is_128k(self) -> bool {
        matches!(self, FlashChip::Sanyo128 | FlashChip::Macronix128)
    }

    /// Erase sector granularity: 4KB for all GBATEK 64K/128K types
    /// except Atmel (512x128-byte sectors).
    pub fn sector_size(self) -> usize {
        match self {
            FlashChip::Atmel64 => 128,
            _ => 0x1000,
        }
    }
}

#[derive(Debug)]
pub struct FlashSave {
    data: Vec<u8>,
    is_128k: bool,
    chip: FlashChip,
    bank: usize, // 0 or 1 for 128K — 現在アクティブな64KBバンク
    state: FlashState,
    id_mode: bool,
}

impl FlashSave {
    pub fn new(is_128k: bool) -> Self {
        let size = if is_128k { 0x20000 } else { 0x10000 };
        let chip = if is_128k {
            FlashChip::Sanyo128
        } else {
            FlashChip::Panasonic64
        };
        Self {
            data: vec![0xFF; size],
            is_128k,
            chip,
            bank: 0,
            state: FlashState::Ready,
            id_mode: false,
        }
    }

    /// Select the emulated Flash chip (e.g. from settings). Only
    /// size-compatible chips are accepted; anything else is ignored so
    /// the backend can never disagree with its storage size.
    pub fn set_chip(&mut self, chip: FlashChip) {
        if chip.is_128k() == self.is_128k {
            self.chip = chip;
        }
    }

    pub fn chip(&self) -> FlashChip {
        self.chip
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
            // GBATEK Device Types (MSB=device, LSB=manufacturer).
            let manufacturer = self.chip.manufacturer();
            let device = self.chip.device();
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
        // GBATEK Flash command sequences. Every command needs the
        // AA@5555, 55@2AAA unlock; erase additionally needs the 80 setup
        // plus a second unlock; program/bank-switch consume the next write
        // as data. Anything else aborts back to Ready.
        match self.state {
            FlashState::Ready => self.accept_unlock(low, byte),
            FlashState::Unlock1 => self.accept_unlock2(low, byte),
            FlashState::Unlock2 => self.execute_command(byte),
            FlashState::ProgramArmed => self.finish_program(addr, byte),
            FlashState::BankArmed => self.finish_bank_switch(addr, byte),
            FlashState::EraseSetup => self.accept_erase_unlock(low, byte),
            FlashState::EraseUnlock1 => self.accept_erase_unlock2(low, byte),
            FlashState::EraseUnlock2 => self.execute_erase(addr, byte),
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
        let mut out = Vec::with_capacity(self.data.len() + 5);
        out.extend_from_slice(&self.data);
        out.push(self.bank as u8);
        out.push(u8::from(self.id_mode));
        out.push(self.state as u8);
        out.push(if self.is_128k { 1 } else { 0 });
        out.push(self.chip as u8);
        out
    }

    fn deserialize_state(&mut self, data: &[u8]) -> Result<(), String> {
        // Current format appends one chip byte; older states (data+4)
        // fall back to the size-default chip.
        if data.len() != self.data.len() + 5 && data.len() != self.data.len() + 4 {
            return Err(format!("Flash state size mismatch: {}", data.len()));
        }
        let (ram, tail) = data.split_at(self.data.len());
        self.data.copy_from_slice(ram);
        self.bank = usize::from(tail[0]) & 1;
        self.id_mode = tail[1] != 0;
        self.state = match tail[2] {
            1 => FlashState::Unlock1,
            2 => FlashState::Unlock2,
            3 => FlashState::ProgramArmed,
            4 => FlashState::BankArmed,
            5 => FlashState::EraseSetup,
            6 => FlashState::EraseUnlock1,
            7 => FlashState::EraseUnlock2,
            _ => FlashState::Ready,
        };
        if tail.len() == 5 {
            let chip = match tail[4] {
                1 => FlashChip::Sst64,
                2 => FlashChip::Macronix64,
                3 => FlashChip::Atmel64,
                4 => FlashChip::Sanyo128,
                5 => FlashChip::Macronix128,
                _ => FlashChip::Panasonic64,
            };
            if chip.is_128k() == self.is_128k {
                self.chip = chip;
            }
        }
        Ok(())
    }
}

impl FlashSave {
    fn finish_program(&mut self, address: u32, value: u8) {
        self.state = FlashState::Ready;
        let offset = (address & 0xFFFF) as usize + self.bank_offset();
        if let Some(byte) = self.data.get_mut(offset) {
            *byte &= value;
        }
    }

    fn finish_bank_switch(&mut self, address: u32, value: u8) {
        self.state = FlashState::Ready;
        if self.is_128k && address == 0x0E000000 && value <= 1 {
            self.bank = usize::from(value);
        }
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

    fn execute_command(&mut self, command: u8) {
        // Reached only after the AA@5555, 55@2AAA unlock sequence.
        // ID-mode exit (F0) likewise requires the unlock (a bare F0 only
        // terminates Macronix write/erase on timeout, which is unmodeled).
        self.state = match command {
            0x90 => {
                self.id_mode = true;
                FlashState::Ready
            }
            0xF0 => {
                self.id_mode = false;
                FlashState::Ready
            }
            0x80 => FlashState::EraseSetup,
            0xA0 => FlashState::ProgramArmed,
            0xB0 if self.is_128k => FlashState::BankArmed,
            _ => FlashState::Ready,
        };
    }

    fn accept_erase_unlock(&mut self, low: u32, value: u8) {
        self.state = if low == 0x5555 && value == 0xAA {
            FlashState::EraseUnlock1
        } else {
            FlashState::Ready
        };
    }

    fn accept_erase_unlock2(&mut self, low: u32, value: u8) {
        self.state = if low == 0x2AAA && value == 0x55 {
            FlashState::EraseUnlock2
        } else {
            FlashState::Ready
        };
    }

    fn execute_erase(&mut self, address: u32, command: u8) {
        // Reached only after 80 setup plus the second unlock sequence.
        self.state = FlashState::Ready;
        match command {
            0x10 => self.data.fill(0xFF),
            0x30 => self.erase_sector(address),
            _ => {}
        }
    }

    fn erase_sector(&mut self, address: u32) {
        let sector = self.chip.sector_size();
        let start = ((address & 0xFFFF) as usize & !(sector - 1)) + self.bank_offset();
        let end = (start + sector).min(self.data.len());
        self.data[start..end].fill(0xFF);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flash128_reports_sanyo_id() {
        // GBATEK Device Types: 1362h Sanyo 128K (device 13h, maker 62h).
        let mut flash = FlashSave::new(true);
        flash.write(0x0E005555, 1, 0xAA);
        flash.write(0x0E002AAA, 1, 0x55);
        flash.write(0x0E005555, 1, 0x90);
        assert_eq!(flash.read(0x0E000000, 1), 0x62);
        assert_eq!(flash.read(0x0E000001, 1), 0x13);
        // ID-mode exit requires the unlock sequence; a bare F0 is a no-op.
        flash.write(0x0E000000, 1, 0xF0);
        assert_eq!(flash.read(0x0E000000, 1), 0x62);
        flash.write(0x0E005555, 1, 0xAA);
        flash.write(0x0E002AAA, 1, 0x55);
        flash.write(0x0E005555, 1, 0xF0);
        assert_eq!(flash.read(0x0E000000, 1), 0xFF);
    }

    #[test]
    fn flash64_reports_panasonic_id() {
        let mut flash = FlashSave::new(false);
        flash.write(0x0E005555, 1, 0xAA);
        flash.write(0x0E002AAA, 1, 0x55);
        flash.write(0x0E005555, 1, 0x90);
        assert_eq!(flash.read(0x0E000000, 1), 0x32);
        assert_eq!(flash.read(0x0E000001, 1), 0x1B);
    }

    #[test]
    fn all_gbatek_chip_ids_reported() {
        // GBATEK FlashROM Device Types, MSB=device LSB=manufacturer.
        let cases = [
            (false, FlashChip::Panasonic64, 0x32, 0x1B),
            (false, FlashChip::Sst64, 0xBF, 0xD4),
            (false, FlashChip::Macronix64, 0xC2, 0x1C),
            (false, FlashChip::Atmel64, 0x1F, 0x3D),
            (true, FlashChip::Sanyo128, 0x62, 0x13),
            (true, FlashChip::Macronix128, 0xC2, 0x09),
        ];
        for (is_128k, chip, maker, device) in cases {
            let mut flash = FlashSave::new(is_128k);
            flash.set_chip(chip);
            assert_eq!(flash.chip(), chip);
            flash.write(0x0E005555, 1, 0xAA);
            flash.write(0x0E002AAA, 1, 0x55);
            flash.write(0x0E005555, 1, 0x90);
            assert_eq!(flash.read(0x0E000000, 1), maker, "{chip:?}");
            assert_eq!(flash.read(0x0E000001, 1), device, "{chip:?}");
        }
    }

    #[test]
    fn set_chip_rejects_size_mismatch() {
        let mut flash = FlashSave::new(false);
        flash.set_chip(FlashChip::Sanyo128);
        assert_eq!(flash.chip(), FlashChip::Panasonic64);
    }

    #[test]
    fn atmel_erases_128b_sectors() {
        let mut flash = FlashSave::new(false);
        flash.set_chip(FlashChip::Atmel64);
        flash.data[0x100] = 0x00;
        flash.data[0x180] = 0x00;
        flash.write(0x0E005555, 1, 0xAA);
        flash.write(0x0E002AAA, 1, 0x55);
        flash.write(0x0E005555, 1, 0x80);
        flash.write(0x0E005555, 1, 0xAA);
        flash.write(0x0E002AAA, 1, 0x55);
        // 0x100 is inside sector 0x100-0x17F; 0x180 must survive.
        flash.write(0x0E000100, 1, 0x30);
        assert_eq!(flash.data[0x100], 0xFF);
        assert_eq!(flash.data[0x180], 0x00);
    }

    #[test]
    fn erase_requires_80_setup_sequence() {
        let mut flash = FlashSave::new(false);
        flash.data[0x1000] = 0x00;
        // AA,55,30 without the 80 setup + second unlock must not erase.
        flash.write(0x0E005555, 1, 0xAA);
        flash.write(0x0E002AAA, 1, 0x55);
        flash.write(0x0E001000, 1, 0x30);
        assert_eq!(flash.data[0x1000], 0x00);
        // Chip erase without setup likewise does nothing.
        flash.write(0x0E005555, 1, 0xAA);
        flash.write(0x0E002AAA, 1, 0x55);
        flash.write(0x0E005555, 1, 0x10);
        assert_eq!(flash.data[0x1000], 0x00);
    }

    #[test]
    fn program_requires_a0_prefix() {
        let mut flash = FlashSave::new(false);
        // AA,55,<data> directly must leave flash unchanged.
        flash.write(0x0E005555, 1, 0xAA);
        flash.write(0x0E002AAA, 1, 0x55);
        flash.write(0x0E000000, 1, 0x00);
        assert_eq!(flash.data[0], 0xFF);
    }

    #[test]
    fn bank_switch_ignored_on_64k() {
        let mut flash = FlashSave::new(false);
        flash.write(0x0E005555, 1, 0xAA);
        flash.write(0x0E002AAA, 1, 0x55);
        flash.write(0x0E005555, 1, 0xB0);
        flash.write(0x0E000000, 1, 0x01);
        assert_eq!(flash.bank, 0);
    }

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
