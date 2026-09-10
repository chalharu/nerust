/// GBA APU - Phase 9
/// Handles GBA sound registers 0x04000060-0x0400009F and wave RAM.
/// PSG/FIFO mixing is still stubbed, but registers are now owned here instead of GbaMemoryBus latch.
/// FIFO_A/B (0x040000A0/A4) are 32-byte streaming buffers fed by DMA
/// Special (DMA1/DMA2, 4x32-bit bursts); without a sound backend they are
/// stored but never drained (no periodic drain is modeled).
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
    /// Two 16-byte wave banks (GBATEK NR30): bit 6 selects the PLAYING
    /// bank while CPU access addresses the other bank.
    pub wave_ram: Box<[u8; 0x20]>,
    pub fifo_a: std::collections::VecDeque<u8>,
    pub fifo_b: std::collections::VecDeque<u8>,
    /// Minimal sound-driver HLE state (GBATEK BIOS Sound Functions).
    pub sound_area: u32,
    pub sound_mode: u32,
    pub sound_vsync_enabled: bool,
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
            fifo_a: std::collections::VecDeque::with_capacity(32),
            fifo_b: std::collections::VecDeque::with_capacity(32),
            sound_area: 0,
            sound_mode: 0,
            sound_vsync_enabled: false,
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
        self.fifo_a.clear();
        self.fifo_b.clear();
    }

    /// Remaining bytes in a DirectSound FIFO.
    pub fn fifo_len(&self, fifo_b: bool) -> usize {
        if fifo_b {
            self.fifo_b.len()
        } else {
            self.fifo_a.len()
        }
    }

    /// Move one byte from FIFO to the (unmodeled) DAC on timer overflow
    /// (GBATEK "DMA-Sound Playback Procedure").
    pub fn drain_fifo(&mut self, fifo_b: bool) {
        let fifo = if fifo_b {
            &mut self.fifo_b
        } else {
            &mut self.fifo_a
        };
        let _ = fifo.pop_front();
    }

    /// SOUNDCNT_H write: store the value and honor the FIFO reset bits
    /// (bit 11 = reset FIFO A, bit 15 = reset FIFO B).
    pub fn write_soundcnt_hi(&mut self, value: u16) {
        self.soundcnt_hi = value;
        if value & (1 << 11) != 0 {
            self.fifo_a.clear();
        }
        if value & (1 << 15) != 0 {
            self.fifo_b.clear();
        }
    }

    /// SOUNDCNT_X write (GBATEK NR52 + mGBA io.c `value & 0x0080`): only
    /// bit 7 is R/W (bits 0-3 are read-only channel flags our PSG does not
    /// track, so they read 0). Clearing a set master enable resets the PSG
    /// range 4000060h..4000081h; 4000082h/4000088h are kept.
    /// (SOUNDCNT_H bits 11/15 stay stored: GBATEK marks them "W?" and the
    /// ROM-pinned 0xDA0C readback forbids masking.)
    pub fn write_soundcnt_x(&mut self, value: u16) {
        if value & 0x80 == 0 && self.soundcnt_x & 0x80 != 0 {
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
            self.fifo_a.clear();
            self.fifo_b.clear();
        }
        self.soundcnt_x = value & 0x80;
    }

    /// Wave RAM CPU access (GBATEK NR30): the CPU sees the bank NOT
    /// selected for playback (bit 6). `aligned` is the 0x90-0x9E address.
    fn wave_cpu_base(&self) -> usize {
        if self.sound3cnt_lo & (1 << 6) != 0 {
            0
        } else {
            16
        }
    }

    pub fn wave_read(&self, aligned: u32) -> u16 {
        let base = self.wave_cpu_base() + ((aligned & 0xF) as usize);
        u16::from_le_bytes([self.wave_ram[base], self.wave_ram[base + 1]])
    }

    pub fn wave_write(&mut self, aligned: u32, value: u16) {
        let base = self.wave_cpu_base() + ((aligned & 0xF) as usize);
        self.wave_ram[base] = (value & 0xFF) as u8;
        self.wave_ram[base + 1] = (value >> 8) as u8;
    }

    /// Push bytes into a DirectSound FIFO (max 32 bytes; overflow is dropped,
    /// approximating full-FIFO HW where extra writes have no effect).
    /// `bytes` are appended LSB-first from `value` for `width` bytes.
    pub fn push_fifo(&mut self, fifo_b: bool, value: u32, width: u8) {
        let fifo = if fifo_b {
            &mut self.fifo_b
        } else {
            &mut self.fifo_a
        };
        for i in 0..width.min(4) {
            if fifo.len() >= 32 {
                break;
            }
            fifo.push_back((value >> (i * 8)) as u8);
        }
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
            // FIFO_A/B (A0/A4) are write-only streaming buffers; reads are open bus.
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
            // FIFO handled by the bus (needs byte-lane info); ignore here.
            _ => return false,
        }
        true
    }
}
