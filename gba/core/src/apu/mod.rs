/// GBA APU owning sound registers, wave RAM and DirectSound FIFOs.
/// PSG/FIFO mixing is stubbed; FIFOs stream via timer-driven DMA refill.
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

    /// SOUNDCNT_H write: FIFO reset bits (11/15) act on the written value,
    /// then the R/W mask 0x770F is stored (mGBA GBAIOWrite `value &= 0x770F`;
    /// GBATEK marks 11/15 "W?", and mgba-suite io-read pins write-0xFFFF ->
    /// 0x770F, i.e. both reset bits read 0). SOUND1CNT_LO..SOUNDCNT_LO use
    /// the same write-time masking at the bus (see `GbaMemoryBus` writes).
    pub fn write_soundcnt_hi(&mut self, value: u16) {
        if value & (1 << 11) != 0 {
            self.fifo_a.clear();
        }
        if value & (1 << 15) != 0 {
            self.fifo_b.clear();
        }
        self.soundcnt_hi = value & 0x770F;
    }

    /// SOUNDCNT_X write: only bit 7 is R/W; clearing master enable resets PSG state.
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
            self.soundcnt_hi &= 0xFF00;
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
            0x04000090 => self.wave_read(0x90),
            0x04000092 => self.wave_read(0x92),
            0x04000094 => self.wave_read(0x94),
            0x04000096 => self.wave_read(0x96),
            0x04000098 => self.wave_read(0x98),
            0x0400009A => self.wave_read(0x9A),
            0x0400009C => self.wave_read(0x9C),
            0x0400009E => self.wave_read(0x9E),
            // FIFO_A/B (A0/A4) are write-only streaming buffers; reads are open bus.
            _ => return None,
        })
    }

    pub fn write(&mut self, addr: u32, value: u16) -> bool {
        // Write-time R/W masks (mGBA GBAIOWrite; GBATEK R/W maps).
        // Unreadable bits never persist, so reads return the stored value.
        match addr {
            0x04000060 => self.sound1cnt_lo = value & 0x007F,
            0x04000062 => self.sound1cnt_hi = value & 0xFFC0,
            0x04000064 => self.sound1cnt_x = value & 0x4000,
            0x04000068 => self.sound2cnt_lo = value & 0xFFC0,
            0x0400006C => self.sound2cnt_hi = value & 0x4000,
            0x04000070 => self.sound3cnt_lo = value & 0x00E0,
            0x04000072 => self.sound3cnt_hi = value & 0xE000,
            0x04000074 => self.sound3cnt_x = value & 0x4000,
            0x04000078 => self.sound4cnt_lo = value & 0xFF00,
            0x0400007C => self.sound4cnt_hi = value & 0x40FF,
            0x04000080 => self.soundcnt_lo = value & 0xFF77,
            0x04000082 => self.write_soundcnt_hi(value),
            0x04000084 => self.write_soundcnt_x(value),
            0x04000088 => self.soundbias = value & 0xC3FE,
            0x04000090 => self.wave_write(0x90, value),
            0x04000092 => self.wave_write(0x92, value),
            0x04000094 => self.wave_write(0x94, value),
            0x04000096 => self.wave_write(0x96, value),
            0x04000098 => self.wave_write(0x98, value),
            0x0400009A => self.wave_write(0x9A, value),
            0x0400009C => self.wave_write(0x9C, value),
            0x0400009E => self.wave_write(0x9E, value),
            // FIFO handled by the bus (needs byte-lane info); ignore here.
            _ => return false,
        }
        true
    }
}
