/// GBA APU owning sound registers, wave RAM and DirectSound FIFOs.
/// Four PSG channels (sweep/duty/envelope/length timers, wave table,
/// noise LFSR) mix with the two timer-clocked FIFO DAC latches at the
/// 32.768kHz native grid; the console drains device-rate samples per frame.
pub mod psg;

use nerust_core_traits::audio::StereoSample;

use self::psg::{Noise, Square, Wave};
use crate::bios::sound_driver::DriverVoice;

/// Native DAC grid: 16.78MHz / 512.
pub const MIX_RATE: u32 = 32_768;
const T_CYCLES_PER_MIX: u64 = 512;
/// Frame sequencer: 512Hz steps (32768 T-cycles each).
const T_CYCLES_PER_SEQ_STEP: u64 = 32_768;

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
    /// BIOS driver voice runtime (vchn[] register side lives in SoundArea).
    pub driver_voices: [DriverVoice; 12],
    /// PSG voices.
    sq1: Square,
    sq2: Square,
    wave: Wave,
    noise: Noise,
    /// FIFO DAC latches (signed 8-bit, clocked by timer overflow).
    dac_a: i8,
    dac_b: i8,
    /// Frame-sequencer position (0-7) and countdowns.
    seq_step: u8,
    seq_timer: u64,
    mix_timer: u64,
    /// Native-grid stereo mix buffer (f32 pairs at 32.768kHz).
    mix_buffer: Vec<(f32, f32)>,
    /// W-only voice latches (length/duty/frequency never read back;
    /// the stored registers keep readable bits only, mGBA-masked).
    freq1: u16,
    freq2: u16,
    freq3: u16,
    len1: u8,
    len2: u8,
    len3: u16,
    len4: u8,
    duty1: u8,
    duty2: u8,
    /// Device-rate resample cursor over the grid timeline.
    rs_pos: f64,
    rs_prev: (f32, f32),
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
            driver_voices: [DriverVoice::default(); 12],
            sq1: Square::default(),
            sq2: Square::default(),
            wave: Wave::default(),
            noise: Noise::default(),
            dac_a: 0,
            dac_b: 0,
            seq_step: 0,
            seq_timer: T_CYCLES_PER_SEQ_STEP,
            mix_timer: T_CYCLES_PER_MIX,
            mix_buffer: Vec::new(),
            freq1: 0,
            freq2: 0,
            freq3: 0,
            len1: 64,
            len2: 64,
            len3: 255,
            len4: 64,
            duty1: 0,
            duty2: 0,
            rs_pos: 0.0,
            rs_prev: (0.0, 0.0),
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
        self.sq1 = Square::default();
        self.sq2 = Square::default();
        self.wave = Wave::default();
        self.noise = Noise::default();
        self.dac_a = 0;
        self.dac_b = 0;
        self.seq_step = 0;
        self.seq_timer = T_CYCLES_PER_SEQ_STEP;
        self.mix_timer = T_CYCLES_PER_MIX;
        self.mix_buffer.clear();
        self.freq1 = 0;
        self.freq2 = 0;
        self.freq3 = 0;
        self.len1 = 64;
        self.len2 = 64;
        self.len3 = 255;
        self.len4 = 64;
        self.duty1 = 0;
        self.duty2 = 0;
        self.rs_pos = 0.0;
        self.rs_prev = (0.0, 0.0);
    }

    /// Remaining bytes in a DirectSound FIFO.
    pub fn fifo_len(&self, fifo_b: bool) -> usize {
        if fifo_b {
            self.fifo_b.len()
        } else {
            self.fifo_a.len()
        }
    }

    /// Move one byte from FIFO to the DAC latch on timer overflow
    /// (GBATEK "DMA-Sound Playback Procedure").
    pub fn drain_fifo(&mut self, fifo_b: bool) {
        let (fifo, dac) = if fifo_b {
            (&mut self.fifo_b, &mut self.dac_b)
        } else {
            (&mut self.fifo_a, &mut self.dac_a)
        };
        if let Some(byte) = fifo.pop_front() {
            *dac = byte as i8;
        }
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
        if value & 0x80 != 0 && self.soundcnt_x & 0x80 == 0 {
            // mGBA master-enable: the frame counter restarts at 7, so the
            // first post-enable step is a length step.
            self.seq_step = 7;
        }
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
            self.dac_a = 0;
            self.dac_b = 0;
            self.sq1.core.active = false;
            self.sq2.core.active = false;
            self.wave.active = false;
            self.noise.core.active = false;
        }
        self.soundcnt_x = value & 0x80;
    }

    /// NR30 write: bit 7 off stops the wave channel at once (GBATEK).
    pub fn write_sound3cnt_lo(&mut self, value: u16) {
        self.sound3cnt_lo = value & 0x00E0;
        if value & (1 << 7) == 0 {
            self.wave.active = false;
        }
    }

    /// NR10 write: sweep staging (live zombie rule).
    pub fn write_sound1cnt_lo(&mut self, value: u16) {
        self.sound1cnt_lo = value & 0x007F;
        self.sq1.write_sweep((value & 0x7F) as u8);
    }

    /// NR11/12 write: length + duty latch (W-only), envelope stays live.
    /// Envelope 0 (bits 11-15 clear) powers the DAC off and stops the
    /// channel at once (Pan Docs DAC power; mGBA `_writeEnvelope` false).
    pub fn write_sound1cnt_hi(&mut self, value: u16) {
        self.sound1cnt_hi = value & 0xFFC0;
        self.len1 = 64 - (value & 0x3F) as u8;
        self.duty1 = ((value >> 6) & 3) as u8;
        if value & 0xF800 == 0 {
            self.sq1.core.active = false;
        }
    }

    /// NR13/14 write: frequency latch, restart on bit 15 (W-only).
    pub fn write_sound1cnt_x(&mut self, value: u16) {
        self.freq1 = value & 0x7FF;
        self.sound1cnt_x = value & 0x4000;
        if value & 0x8000 != 0 {
            self.trigger_ch1();
        }
    }

    pub fn write_sound2cnt_lo(&mut self, value: u16) {
        self.sound2cnt_lo = value & 0xFFC0;
        self.len2 = 64 - (value & 0x3F) as u8;
        self.duty2 = ((value >> 6) & 3) as u8;
        if value & 0xF800 == 0 {
            self.sq2.core.active = false;
        }
    }

    pub fn write_sound2cnt_hi(&mut self, value: u16) {
        self.freq2 = value & 0x7FF;
        self.sound2cnt_hi = value & 0x4000;
        if value & 0x8000 != 0 {
            self.trigger_ch2();
        }
    }

    /// NR31 write: wave length latch (W-only).
    pub fn write_sound3cnt_hi(&mut self, value: u16) {
        self.sound3cnt_hi = value & 0xE000;
        self.len3 = 256 - (value & 0xFF);
    }

    /// NR33/34 write: rate latch, restart on bit 15 (W-only).
    pub fn write_sound3cnt_x(&mut self, value: u16) {
        self.freq3 = value & 0x7FF;
        self.sound3cnt_x = value & 0x4000;
        if value & 0x8000 != 0 {
            self.trigger_ch3();
        }
    }

    /// NR41 write: noise length latch (W-only); envelope 0 kills DAC.
    pub fn write_sound4cnt_lo(&mut self, value: u16) {
        self.sound4cnt_lo = value & 0xFF00;
        self.len4 = 64 - (value & 0x3F) as u8;
        if value & 0xF800 == 0 {
            self.noise.core.active = false;
        }
    }

    /// NR44 write: restart on bit 15 (W-only).
    pub fn write_sound4cnt_hi(&mut self, value: u16) {
        self.sound4cnt_hi = value & 0x40FF;
        if value & 0x8000 != 0 {
            self.trigger_ch4();
        }
    }

    /// Channel restarts (NRx4 bit 15). Envelope/length/sweep state reload
    /// from the staged registers.
    pub fn trigger_ch1(&mut self) {
        let env = self.sound1cnt_hi;
        self.sq1.write_sweep((self.sound1cnt_lo & 0x7F) as u8);
        self.sq1.trigger(
            self.freq1,
            self.len1,
            (env >> 12) as u8 & 0xF,
            env,
            self.seq_step & 1 != 0,
        );
        // Dead envelope (DAC off) never starts the voice.
        if env & 0xF800 == 0 {
            self.sq1.core.active = false;
        }
    }

    pub fn trigger_ch2(&mut self) {
        let env = self.sound2cnt_lo;
        self.sq2.trigger(
            self.freq2,
            self.len2,
            (env >> 12) as u8 & 0xF,
            env,
            self.seq_step & 1 != 0,
        );
        if env & 0xF800 == 0 {
            self.sq2.core.active = false;
        }
    }

    pub fn trigger_ch3(&mut self) {
        self.wave.bank = if self.sound3cnt_lo & (1 << 6) != 0 {
            1
        } else {
            0
        };
        self.wave.trigger(
            self.len3,
            self.sound3cnt_lo & (1 << 5) != 0,
            self.seq_step & 1 != 0,
        );
        if self.sound3cnt_lo & (1 << 7) == 0 {
            self.wave.active = false;
        }
    }

    pub fn trigger_ch4(&mut self) {
        let env = self.sound4cnt_lo;
        self.noise.trigger(
            self.len4,
            (env >> 12) as u8 & 0xF,
            env,
            self.sound4cnt_hi & (1 << 3) != 0,
            self.seq_step & 1 != 0,
        );
        if env & 0xF800 == 0 {
            self.noise.core.active = false;
        }
    }

    /// SOUNDCNT_X read value: bit 7 master flag plus live channel status
    /// bits 0-3 (GBATEK: set on start, cleared on length expiry).
    pub fn soundcnt_x_read(&self) -> u16 {
        (self.soundcnt_x & 0x80)
            | (u16::from(self.sq1.core.active))
            | (u16::from(self.sq2.core.active) << 1)
            | (u16::from(self.wave.active) << 2)
            | (u16::from(self.noise.core.active) << 3)
    }

    /// Advance channel timers, the 512Hz frame sequencer and the native
    /// mix grid by one CPU T-cycle. Returns true when a grid sample was
    /// pushed (driver voices fold into the tail on the bus side).
    pub fn tick(&mut self) -> bool {
        if self.soundcnt_x & 0x80 != 0 {
            self.sq1.tick_timer(self.freq1, true);
            self.sq2.tick_timer(self.freq2, false);
            self.wave.tick_timer(self.freq3);
            let r = (self.sound4cnt_hi & 7) as u8;
            let s = ((self.sound4cnt_hi >> 4) & 7) as u8;
            self.noise.tick_timer(r, s);
        }
        self.seq_timer -= 1;
        if self.seq_timer == 0 {
            self.seq_timer = T_CYCLES_PER_SEQ_STEP;
            // mGBA UpdateFrame: the sequencer only advances while the
            // master is enabled (period timing still free-runs).
            if self.soundcnt_x & 0x80 != 0 {
                self.seq_step = (self.seq_step + 1) & 7;
                self.tick_sequencer();
            }
        }
        self.mix_timer -= 1;
        if self.mix_timer == 0 {
            self.mix_timer = T_CYCLES_PER_MIX;
            let (l, r) = self.mix_grid();
            self.mix_buffer.push((l, r));
            return true;
        }
        false
    }

    fn tick_sequencer(&mut self) {
        let step = self.seq_step;
        if step & 1 == 0 {
            let l1 = self.sound1cnt_x & (1 << 14) != 0;
            let l2 = self.sound2cnt_hi & (1 << 14) != 0;
            let l3 = self.sound3cnt_x & (1 << 14) != 0;
            let l4 = self.sound4cnt_hi & (1 << 14) != 0;
            self.sq1.core.tick_length(l1, 64);
            self.sq2.core.tick_length(l2, 64);
            self.wave.tick_length(l3, 256);
            self.noise.core.tick_length(l4, 64);
        }
        if step == 2 || step == 6 {
            self.sq1.tick_sweep();
        }
        if step == 7 {
            self.sq1.core.tick_envelope(self.sound1cnt_hi);
            self.sq2.core.tick_envelope(self.sound2cnt_lo);
            self.noise.core.tick_envelope(self.sound4cnt_lo);
        }
    }

    /// One native-grid stereo sample. Integer 10-bit mix after NBA:
    /// PSG voices scaled by SOUNDCNT_H/L, FIFO latches x2/x4, plus bias,
    /// clipped to 0..0x3FF and centered.
    fn mix_grid(&mut self) -> (f32, f32) {
        if self.soundcnt_x & 0x80 == 0 {
            let bias = f32::from((self.soundbias >> 1) & 0x1FF);
            let out = (bias - 512.0) / 512.0;
            return (out, out);
        }
        let psg_mul = [1i32, 2, 4, 4][(self.soundcnt_hi & 3) as usize];
        let l_vol = ((self.soundcnt_lo >> 4) & 7) as i32 + 1;
        let r_vol = (self.soundcnt_lo & 7) as i32 + 1;
        let outs = [
            self.sq1.output(self.duty1) as i32,
            self.sq2.output(self.duty2) as i32,
            self.wave.output(
                &self.wave_ram,
                ((self.sound3cnt_hi >> 13) & 3) as u8,
                self.sound3cnt_hi & (1 << 15) != 0,
            ) as i32,
            self.noise.output() as i32,
        ];
        let en_l = (self.soundcnt_lo >> 12) & 0xF;
        let en_r = (self.soundcnt_lo >> 8) & 0xF;
        let mut sum_l = 0i32;
        let mut sum_r = 0i32;
        for (i, out) in outs.iter().enumerate() {
            if en_l & (1 << i) != 0 {
                sum_l += out;
            }
            if en_r & (1 << i) != 0 {
                sum_r += out;
            }
        }
        // (master+1)>>5 after NBA: voices x mul x (vol) >> 5.
        sum_l = (sum_l * psg_mul * l_vol) >> 5;
        sum_r = (sum_r * psg_mul * r_vol) >> 5;
        let fifo_gain = |fifo: bool| {
            if fifo {
                if self.soundcnt_hi & (1 << 3) != 0 {
                    4
                } else {
                    2
                }
            } else if self.soundcnt_hi & (1 << 2) != 0 {
                4
            } else {
                2
            }
        };
        let a_on_l = self.soundcnt_hi & (1 << 8) != 0;
        let a_on_r = self.soundcnt_hi & (1 << 9) != 0;
        let b_on_l = self.soundcnt_hi & (1 << 12) != 0;
        let b_on_r = self.soundcnt_hi & (1 << 13) != 0;
        let da = i32::from(self.dac_a) * fifo_gain(false);
        let db = i32::from(self.dac_b) * fifo_gain(true);
        if a_on_l {
            sum_l += da;
        }
        if a_on_r {
            sum_r += da;
        }
        if b_on_l {
            sum_l += db;
        }
        if b_on_r {
            sum_r += db;
        }
        let bias = i32::from((self.soundbias >> 1) & 0x1FF);
        let to_f32 = |sum: i32| (sum + bias).clamp(0, 0x3FF) as f32 / 512.0 - 1.0;
        (to_f32(sum_l), to_f32(sum_r))
    }

    /// Drain the grid buffer as device-rate samples (linear interpolation
    /// over the 32.768kHz timeline; cursor persists across frames).
    pub fn drain_resampled(&mut self, rate: u32) -> Vec<StereoSample> {
        if self.mix_buffer.is_empty() || rate == 0 {
            return Vec::new();
        }
        let step = f64::from(MIX_RATE) / f64::from(rate);
        let mut out = Vec::new();
        // Position relative to the current buffer head.
        let mut pos = self.rs_pos;
        let buf = &self.mix_buffer;
        while (pos as usize) + 1 < buf.len() {
            let i = pos as usize;
            let frac = (pos - i as f64) as f32;
            let (l0, r0) = if i == 0 { self.rs_prev } else { buf[i - 1] };
            let (l1, r1) = buf[i];
            out.push(StereoSample {
                left: l0 + (l1 - l0) * frac,
                right: r0 + (r1 - r0) * frac,
            });
            pos += step;
        }
        // Carry the tail for the next drain: the sample just before the
        // new head stays as the interpolation predecessor.
        let keep_from = (pos as usize).saturating_sub(1);
        if keep_from > 0 {
            self.rs_prev = buf[keep_from - 1];
        }
        self.rs_pos = pos - keep_from as f64;
        self.mix_buffer.drain(..keep_from);
        out
    }
    /// Mutable tail of the grid mix buffer (driver-voice fold-in).
    pub fn mix_tail_mut(&mut self) -> Option<&mut (f32, f32)> {
        self.mix_buffer.last_mut()
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
            0x04000084 => self.soundcnt_x_read(),
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
            0x04000060 => self.write_sound1cnt_lo(value),
            0x04000062 => self.write_sound1cnt_hi(value),
            0x04000064 => self.write_sound1cnt_x(value),
            0x04000068 => self.write_sound2cnt_lo(value),
            0x0400006C => self.write_sound2cnt_hi(value),
            0x04000070 => self.write_sound3cnt_lo(value),
            0x04000072 => self.write_sound3cnt_hi(value),
            0x04000074 => self.write_sound3cnt_x(value),
            0x04000078 => self.write_sound4cnt_lo(value),
            0x0400007C => self.write_sound4cnt_hi(value),
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
