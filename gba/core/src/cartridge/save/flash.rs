use super::helpers::read_slice;
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
        read_slice(&self.data, off, width)
    }

    fn write(&mut self, addr: u32, width: u8, value: u32) {
        let low = (addr & 0xFFFF) as u32;
        let byte = (value & 0xFF) as u8;

        // Program pending (after 0xA0) has priority
        if self.program_pending {
            let off = ((addr & 0xFFFF) as usize) + self.bank_offset();
            if off < self.data.len() {
                self.data[off] &= byte;
            }
            self.program_pending = false;
            return;
        }
        // Bank switch pending (after 0xB0) has priority
        if self.bank_switch_pending && self.is_128k && addr == 0x0E000000 {
            if byte == 0x00 || byte == 0x01 {
                self.bank = (byte & 1) as usize;
            }
            self.bank_switch_pending = false;
            return;
        }

        match self.state {
            FlashState::Ready => {
                if low == 0x5555 && byte == 0xAA {
                    self.state = FlashState::Unlock1;
                } else {
                    // 未対応コマンドは無視
                }
            }
            FlashState::Unlock1 => {
                if low == 0x2AAA && byte == 0x55 {
                    self.state = FlashState::Unlock2;
                } else {
                    self.state = FlashState::Ready;
                }
            }
            FlashState::Unlock2 => {
                match byte {
                    0x90 => {
                        self.id_mode = true;
                        self.state = FlashState::Ready;
                    }
                    0xF0 => {
                        self.id_mode = false;
                        self.state = FlashState::Ready;
                    }
                    0x80 => {
                        // Erase setup — next AA/55/10 or 30
                        self.state = FlashState::Ready;
                        // 簡易: Chip eraseは全FFに
                        // 実際は 80 -> AA -> 55 -> 10 で全消去
                        // Phase 4では簡易実装として 0x80 受信時に次を待たずに全消去しない
                    }
                    0xA0 => {
                        // Byte program setup — next write is data
                        self.program_pending = true;
                        self.state = FlashState::Ready;
                    }
                    0xB0 => {
                        // Bank switch — next write to 0x0E000000 selects bank (0 or 1)
                        self.bank_switch_pending = true;
                        self.state = FlashState::Ready;
                    }
                    0x10 => {
                        // Chip erase (after 80 AA 55)
                        self.data.fill(0xFF);
                        self.state = FlashState::Ready;
                    }
                    0x30 => {
                        // Sector erase 4KB (after 80 AA 55 AA 55)
                        let sector_start =
                            (((addr & 0xFFFF) as usize) & !0xFFF) + self.bank_offset();
                        for i in 0..0x1000 {
                            if sector_start + i < self.data.len() {
                                self.data[sector_start + i] = 0xFF;
                            }
                        }
                        self.state = FlashState::Ready;
                    }
                    _ => {
                        // 0xA0の後のデータ書き込みとして扱う（簡易）
                        let off = ((addr & 0xFFFF) as usize) + self.bank_offset();
                        if off < self.data.len() {
                            self.data[off] &= byte;
                        }
                        self.state = FlashState::Ready;
                    }
                }
                // For A0 program: if width and addr indicate data write, handle
                if width == 1 || width == 2 || width == 4 {
                    // A0後の場合、実際のデータはこのwriteとして既に処理される想定だが
                    // 上記 _ で処理済み
                }
            }
        }

        // A0 byte programの実際のデータ書き込み（簡易: Unlock2後の次のwriteをデータとする）
        // 上記では状態遷移のみで、実際の書き込みは次回呼び出しで処理されるため、
        // ここでは特別な処理なし。Phase 12で精密化。

        // Direct program after A0 (簡易): if state was Unlock2 and byte==A0, next byte is data
        // ただし簡易実装では A0受信後、次回writeでデータとして扱うには追加フラグが必要。
        // Phase 4では A0後の単一バイト書き込みを即時反映する簡易モデルとする:
        // 上記で state=Readyに戻してしまうため、別途 program_pending フラグが必要だが Phase 4では省略。

        let _ = width;
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
