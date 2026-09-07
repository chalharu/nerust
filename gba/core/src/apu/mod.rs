/// GBA APU - Phase 9
/// Handles GBA sound registers 0x04000060-0x040000A6 and wave RAM.
/// PSG/FIFO mixing is still stubbed, but registers are now owned here instead of GbaMemoryBus latch.
#[derive(Debug)]
pub struct GbaApu {
    pub sound1cnt_lo: u16,
    pub sound1cnt_hi: u16,
    pub sound1cnt_x: u16,
    pub sound2cnt_lo: u16,
    pub sound2cnt_hi: u16,
    pub sound3cnt_lo: u16,
    pub sound3cnt_hi: u16,
    pub sound3cnt_x: u16,
    pub sound4cnt_lo: u16,
    pub sound4cnt_hi: u16,
    pub soundcnt_lo: u16,
    pub soundcnt_hi: u16,
    pub soundcnt_x: u16,
    pub soundbias: u16,
    pub wave_ram: Box<[u8; 0x20]>,
}

impl Default for GbaApu {
    fn default() -> Self {
        Self {
            sound1cnt_lo: 0,
            sound1cnt_hi: 0,
            sound1cnt_x: 0,
            sound2cnt_lo: 0,
            sound2cnt_hi: 0,
            sound3cnt_lo: 0,
            sound3cnt_hi: 0,
            sound3cnt_x: 0,
            sound4cnt_lo: 0,
            sound4cnt_hi: 0,
            soundcnt_lo: 0,
            soundcnt_hi: 0,
            soundcnt_x: 0,
            soundbias: 0x200,
            wave_ram: Box::new([0u8; 0x20]),
        }
    }
}

impl GbaApu {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn reset(&mut self) {
        *self = Self::default();
        // mGBA RegisterRamReset SOUND sets bias 0x200 and clears wave
        self.soundbias = 0x200;
        self.wave_ram.fill(0);
    }

    pub fn reset_sound(&mut self) {
        // Called for 0x40 flag (SOUND)
        self.sound1cnt_lo = 0;
        self.sound1cnt_hi = 0;
        self.sound1cnt_x = 0;
        self.sound2cnt_lo = 0;
        self.sound2cnt_hi = 0;
        self.sound3cnt_lo = 0;
        self.sound3cnt_hi = 0;
        self.sound3cnt_x = 0;
        self.sound4cnt_lo = 0;
        self.sound4cnt_hi = 0;
        self.soundcnt_lo = 0;
        self.soundcnt_hi = 0;
        self.soundcnt_x = 0;
        self.soundbias = 0x200;
        self.wave_ram.fill(0);
    }

    pub fn read(&self, addr: u32) -> Option<u16> {
        Some(match addr {
            0x04000060 => self.sound1cnt_lo,
            0x04000062 => self.sound1cnt_hi,
            0x04000064 => self.sound1cnt_x,
            0x04000068 => self.sound2cnt_lo,
            0x0400006C => self.sound2cnt_hi,
            0x04000070 => self.sound3cnt_lo,
            0x04000072 => self.sound3cnt_hi,
            0x04000074 => self.sound3cnt_x,
            0x04000078 => self.sound4cnt_lo,
            0x0400007C => self.sound4cnt_hi,
            0x04000080 => self.soundcnt_lo,
            0x04000082 => self.soundcnt_hi,
            0x04000084 => self.soundcnt_x,
            0x04000088 => self.soundbias,
            0x04000090 => u16::from_le_bytes([self.wave_ram[0], self.wave_ram[1]]),
            0x04000092 => u16::from_le_bytes([self.wave_ram[2], self.wave_ram[3]]),
            0x04000094 => u16::from_le_bytes([self.wave_ram[4], self.wave_ram[5]]),
            0x04000096 => u16::from_le_bytes([self.wave_ram[6], self.wave_ram[7]]),
            0x04000098 => u16::from_le_bytes([self.wave_ram[8], self.wave_ram[9]]),
            0x0400009A => u16::from_le_bytes([self.wave_ram[10], self.wave_ram[11]]),
            0x0400009C => u16::from_le_bytes([self.wave_ram[12], self.wave_ram[13]]),
            0x0400009E => u16::from_le_bytes([self.wave_ram[14], self.wave_ram[15]]),
            0x040000A0 => u16::from_le_bytes([self.wave_ram[16], self.wave_ram[17]]),
            0x040000A2 => u16::from_le_bytes([self.wave_ram[18], self.wave_ram[19]]),
            0x040000A4 => u16::from_le_bytes([self.wave_ram[20], self.wave_ram[21]]),
            0x040000A6 => u16::from_le_bytes([self.wave_ram[22], self.wave_ram[23]]),
            _ => return None,
        })
    }

    pub fn write(&mut self, addr: u32, value: u16) -> bool {
        match addr {
            0x04000060 => self.sound1cnt_lo = value,
            0x04000062 => self.sound1cnt_hi = value,
            0x04000064 => self.sound1cnt_x = value,
            0x04000068 => self.sound2cnt_lo = value,
            0x0400006C => self.sound2cnt_hi = value,
            0x04000070 => self.sound3cnt_lo = value,
            0x04000072 => self.sound3cnt_hi = value,
            0x04000074 => self.sound3cnt_x = value,
            0x04000078 => self.sound4cnt_lo = value,
            0x0400007C => self.sound4cnt_hi = value,
            0x04000080 => self.soundcnt_lo = value,
            0x04000082 => self.soundcnt_hi = value,
            0x04000084 => self.soundcnt_x = value,
            0x04000088 => self.soundbias = value,
            0x04000090 => {
                self.wave_ram[0] = (value & 0xFF) as u8;
                self.wave_ram[1] = (value >> 8) as u8;
            }
            0x04000092 => {
                self.wave_ram[2] = (value & 0xFF) as u8;
                self.wave_ram[3] = (value >> 8) as u8;
            }
            0x04000094 => {
                self.wave_ram[4] = (value & 0xFF) as u8;
                self.wave_ram[5] = (value >> 8) as u8;
            }
            0x04000096 => {
                self.wave_ram[6] = (value & 0xFF) as u8;
                self.wave_ram[7] = (value >> 8) as u8;
            }
            0x04000098 => {
                self.wave_ram[8] = (value & 0xFF) as u8;
                self.wave_ram[9] = (value >> 8) as u8;
            }
            0x0400009A => {
                self.wave_ram[10] = (value & 0xFF) as u8;
                self.wave_ram[11] = (value >> 8) as u8;
            }
            0x0400009C => {
                self.wave_ram[12] = (value & 0xFF) as u8;
                self.wave_ram[13] = (value >> 8) as u8;
            }
            0x0400009E => {
                self.wave_ram[14] = (value & 0xFF) as u8;
                self.wave_ram[15] = (value >> 8) as u8;
            }
            0x040000A0 => {
                self.wave_ram[16] = (value & 0xFF) as u8;
                self.wave_ram[17] = (value >> 8) as u8;
            }
            0x040000A2 => {
                self.wave_ram[18] = (value & 0xFF) as u8;
                self.wave_ram[19] = (value >> 8) as u8;
            }
            0x040000A4 => {
                self.wave_ram[20] = (value & 0xFF) as u8;
                self.wave_ram[21] = (value >> 8) as u8;
            }
            0x040000A6 => {
                self.wave_ram[22] = (value & 0xFF) as u8;
                self.wave_ram[23] = (value >> 8) as u8;
            }
            _ => return false,
        }
        true
    }
}
