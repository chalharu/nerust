/// GBA APU owning sound registers, wave RAM and DirectSound FIFOs.
/// Four PSG channels (sweep/duty/envelope/length timers, wave table,
/// noise LFSR) mix with the two timer-clocked FIFO DAC latches at the
/// 32.768kHz native grid; the console drains device-rate samples per frame.
pub mod psg;

use nerust_core_traits::audio::StereoSample;
use nerust_sound_filter::{Filter, IirFilter};

use self::psg::{Noise, Square, Wave};

/// Runtime voice state of the BIOS sound driver (GBATEK `SoundArea.vchn[]`
/// register side lives in SoundArea RAM). Defined here — next to its owner
/// `GbaApu::driver_voices` — so `apu` never depends on `crate::sound_driver`.
#[derive(Debug, Default, Clone, Copy, serde::Serialize, serde::Deserialize)]
pub struct DriverVoice {
    pub started: bool,
    pub pos: f64,
    pub env: f32,
}

/// Native DAC grid: 16.78MHz / 512.
pub const MIX_RATE: u32 = 32_768;
const T_CYCLES_PER_MIX: u64 = 512;
/// Frame sequencer: 512Hz steps (32768 T-cycles each).
const T_CYCLES_PER_SEQ_STEP: u64 = 32_768;
/// DC-blocking high-pass on the drained device-rate output (first-order,
/// negligible in-band effect; provisional, revisit with measurements in
/// Phase 12). `SimpleDownSampler` does not apply here: the native grid
/// (32.768kHz) sits *below* the device rate (48kHz), i.e. the rate step is
/// an upsample, so only the `IirFilter` half of `nerust_sound_filter` fits.
const OUTPUT_HPF_CUTOFF_HZ: f32 = 20.0;
/// Default device rate for the output HPF (matches `AudioBackend` default).
const OUTPUT_HPF_DEFAULT_RATE: u32 = 48_000;

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
    /// the stored registers keep readable bits only).
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
    /// Stereo DC-block state for the drained output (rebuilt when the
    /// device rate changes; filter memory, excluded from save states).
    output_hpf_l: IirFilter,
    output_hpf_r: IirFilter,
    output_hpf_rate: u32,
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
            output_hpf_l: IirFilter::get_highpass_filter(
                OUTPUT_HPF_DEFAULT_RATE as f32,
                OUTPUT_HPF_CUTOFF_HZ,
            ),
            output_hpf_r: IirFilter::get_highpass_filter(
                OUTPUT_HPF_DEFAULT_RATE as f32,
                OUTPUT_HPF_CUTOFF_HZ,
            ),
            output_hpf_rate: OUTPUT_HPF_DEFAULT_RATE,
        }
    }
}

/// Phase 10 wire state. `mix_buffer` (native-grid content) and the output
/// HPF filter memory are excluded by design: export requires the buffer to
/// hold at most the interpolation tail (see below) and import rebuilds the
/// HPF at the default rate (it re-tracks on the next drain).
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub(crate) struct GbaApuState {
    sound1cnt_lo: u16,
    sound1cnt_hi: u16,
    sound1cnt_x: u16,
    sound2cnt_lo: u16,
    sound2cnt_hi: u16,
    sound3cnt_lo: u16,
    sound3cnt_hi: u16,
    sound3cnt_x: u16,
    sound4cnt_lo: u16,
    sound4cnt_hi: u16,
    soundcnt_lo: u16,
    soundcnt_hi: u16,
    soundcnt_x: u16,
    soundbias: u16,
    wave_ram: serde_bytes::ByteBuf,
    fifo_a: Vec<u8>,
    fifo_b: Vec<u8>,
    sound_area: u32,
    sound_mode: u32,
    sound_vsync_enabled: bool,
    driver_voices: [DriverVoice; 12],
    sq1: Square,
    sq2: Square,
    wave: Wave,
    noise: Noise,
    dac_a: i8,
    dac_b: i8,
    seq_step: u8,
    seq_timer: u64,
    mix_timer: u64,
    freq1: u16,
    freq2: u16,
    freq3: u16,
    len1: u8,
    len2: u8,
    len3: u16,
    len4: u8,
    duty1: u8,
    duty2: u8,
    rs_pos: f64,
    rs_prev: (f32, f32),
}

impl GbaApuState {
    pub(crate) fn validate(&self) -> Result<(), String> {
        if self.wave_ram.len() != 0x20 {
            return Err(format!(
                "apu: wave RAM length wrong: {}",
                self.wave_ram.len()
            ));
        }
        if self.fifo_a.len() > 32 || self.fifo_b.len() > 32 {
            return Err(format!(
                "apu: fifo overflow: {}/{}",
                self.fifo_a.len(),
                self.fifo_b.len()
            ));
        }
        if self.seq_step > 7 {
            return Err(format!("apu: seq_step out of range: {}", self.seq_step));
        }
        if self.seq_timer > T_CYCLES_PER_SEQ_STEP {
            return Err(format!("apu: seq_timer out of range: {}", self.seq_timer));
        }
        if self.mix_timer > T_CYCLES_PER_MIX {
            return Err(format!("apu: mix_timer out of range: {}", self.mix_timer));
        }
        // Latch bounds follow the register write masks.
        if self.freq1 > 0x7FF || self.freq2 > 0x7FF || self.freq3 > 0x7FF {
            return Err("apu: freq latch out of range".to_string());
        }
        if self.len1 > 64 || self.len2 > 64 || self.len4 > 64 || self.len3 > 256 {
            return Err("apu: length latch out of range".to_string());
        }
        if self.duty1 > 3 || self.duty2 > 3 {
            return Err("apu: duty latch out of range".to_string());
        }
        self.sq1.validate()?;
        self.sq2.validate()?;
        self.wave.validate()?;
        self.noise.validate()?;
        for (index, voice) in self.driver_voices.iter().enumerate() {
            if !voice.pos.is_finite() || voice.pos < 0.0 {
                return Err(format!("apu: driver voice {index} bad pos"));
            }
            if !voice.env.is_finite() || !(0.0..=255.0).contains(&voice.env) {
                return Err(format!("apu: driver voice {index} bad env"));
            }
        }
        if !self.rs_pos.is_finite() || self.rs_pos < 0.0 {
            return Err("apu: bad resample cursor".to_string());
        }
        if !self.rs_prev.0.is_finite() || !self.rs_prev.1.is_finite() {
            return Err("apu: bad resample predecessor".to_string());
        }
        Ok(())
    }
}

impl GbaApu {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn reset(&mut self) {
        *self = Self::default();
        // RegisterRamReset SOUND sets bias 0x200 and clears wave
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
        // Fresh filter memory (host-side only); keep the device rate so a
        // non-default backend does not fall back to 48kHz coefficients.
        let rate = self.output_hpf_rate;
        self.output_hpf_l = IirFilter::get_highpass_filter(rate as f32, OUTPUT_HPF_CUTOFF_HZ);
        self.output_hpf_r = IirFilter::get_highpass_filter(rate as f32, OUTPUT_HPF_CUTOFF_HZ);
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
    /// then the R/W mask 0x770F is stored (GBATEK R/W map; both reset bits
    /// read 0, pinned by mgba-suite io-read).
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
    /// channel at once (Pan Docs DAC power).
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
        self.wave.dimension_64 = self.sound3cnt_lo & (1 << 5) != 0;
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
            // The 512Hz sequencer free-runs from boot (no master-off
            // freeze, no enable reset); channels gate individually.
            self.seq_step = (self.seq_step + 1) & 7;
            self.tick_sequencer();
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
            self.sq1.core.tick_length(l1);
            self.sq2.core.tick_length(l2);
            self.wave.tick_length(l3);
            self.noise.core.tick_length(l4);
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

    /// One native-grid stereo sample. Integer 10-bit mix: PSG voices
    /// scaled by SOUNDCNT_H/L, FIFO latches x2/x4, plus bias, clipped to
    /// 0..0x3FF and centered.
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
        // (master+1)>>5: voices x mul x (vol) >> 5.
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

    /// Phase 10 export. `drain_resampled` always leaves the interpolation
    /// tail (1-2 samples) behind; that residue is structural, so it is
    /// truncated deterministically here (`rs_prev` keeps the last sample,
    /// the cursor restarts at the buffer head). Anything larger is mid-frame
    /// content and rejected.
    pub(crate) fn export_state(&self) -> Result<GbaApuState, String> {
        if self.mix_buffer.len() > 2 {
            return Err(format!(
                "apu: mix_buffer not drained: {}",
                self.mix_buffer.len()
            ));
        }
        let rs_prev = self.mix_buffer.last().copied().unwrap_or(self.rs_prev);
        Ok(GbaApuState {
            sound1cnt_lo: self.sound1cnt_lo,
            sound1cnt_hi: self.sound1cnt_hi,
            sound1cnt_x: self.sound1cnt_x,
            sound2cnt_lo: self.sound2cnt_lo,
            sound2cnt_hi: self.sound2cnt_hi,
            sound3cnt_lo: self.sound3cnt_lo,
            sound3cnt_hi: self.sound3cnt_hi,
            sound3cnt_x: self.sound3cnt_x,
            sound4cnt_lo: self.sound4cnt_lo,
            sound4cnt_hi: self.sound4cnt_hi,
            soundcnt_lo: self.soundcnt_lo,
            soundcnt_hi: self.soundcnt_hi,
            soundcnt_x: self.soundcnt_x,
            soundbias: self.soundbias,
            wave_ram: serde_bytes::ByteBuf::from(self.wave_ram.to_vec()),
            fifo_a: self.fifo_a.iter().copied().collect(),
            fifo_b: self.fifo_b.iter().copied().collect(),
            sound_area: self.sound_area,
            sound_mode: self.sound_mode,
            sound_vsync_enabled: self.sound_vsync_enabled,
            driver_voices: self.driver_voices,
            sq1: self.sq1,
            sq2: self.sq2,
            wave: self.wave,
            noise: self.noise,
            dac_a: self.dac_a,
            dac_b: self.dac_b,
            seq_step: self.seq_step,
            seq_timer: self.seq_timer,
            mix_timer: self.mix_timer,
            freq1: self.freq1,
            freq2: self.freq2,
            freq3: self.freq3,
            len1: self.len1,
            len2: self.len2,
            len3: self.len3,
            len4: self.len4,
            duty1: self.duty1,
            duty2: self.duty2,
            rs_pos: 0.0,
            rs_prev,
        })
    }

    pub(crate) fn import_state(&mut self, state: GbaApuState) -> Result<(), String> {
        state.validate()?;
        self.sound1cnt_lo = state.sound1cnt_lo;
        self.sound1cnt_hi = state.sound1cnt_hi;
        self.sound1cnt_x = state.sound1cnt_x;
        self.sound2cnt_lo = state.sound2cnt_lo;
        self.sound2cnt_hi = state.sound2cnt_hi;
        self.sound3cnt_lo = state.sound3cnt_lo;
        self.sound3cnt_hi = state.sound3cnt_hi;
        self.sound3cnt_x = state.sound3cnt_x;
        self.sound4cnt_lo = state.sound4cnt_lo;
        self.sound4cnt_hi = state.sound4cnt_hi;
        self.soundcnt_lo = state.soundcnt_lo;
        self.soundcnt_hi = state.soundcnt_hi;
        self.soundcnt_x = state.soundcnt_x;
        self.soundbias = state.soundbias;
        self.wave_ram.copy_from_slice(&state.wave_ram);
        self.fifo_a = state.fifo_a.into_iter().collect();
        self.fifo_b = state.fifo_b.into_iter().collect();
        self.sound_area = state.sound_area;
        self.sound_mode = state.sound_mode;
        self.sound_vsync_enabled = state.sound_vsync_enabled;
        self.driver_voices = state.driver_voices;
        self.sq1 = state.sq1;
        self.sq2 = state.sq2;
        self.wave = state.wave;
        self.noise = state.noise;
        self.dac_a = state.dac_a;
        self.dac_b = state.dac_b;
        self.seq_step = state.seq_step;
        self.seq_timer = state.seq_timer;
        self.mix_timer = state.mix_timer;
        self.freq1 = state.freq1;
        self.freq2 = state.freq2;
        self.freq3 = state.freq3;
        self.len1 = state.len1;
        self.len2 = state.len2;
        self.len3 = state.len3;
        self.len4 = state.len4;
        self.duty1 = state.duty1;
        self.duty2 = state.duty2;
        self.rs_pos = state.rs_pos;
        self.rs_prev = state.rs_prev;
        // HPF filter memory is excluded from the wire: rebuild at the
        // default rate; the next drain re-tracks the device rate.
        self.mix_buffer.clear();
        self.output_hpf_l =
            IirFilter::get_highpass_filter(OUTPUT_HPF_DEFAULT_RATE as f32, OUTPUT_HPF_CUTOFF_HZ);
        self.output_hpf_r =
            IirFilter::get_highpass_filter(OUTPUT_HPF_DEFAULT_RATE as f32, OUTPUT_HPF_CUTOFF_HZ);
        self.output_hpf_rate = OUTPUT_HPF_DEFAULT_RATE;
        Ok(())
    }

    /// Drain the grid buffer as device-rate samples (linear interpolation
    /// over the 32.768kHz timeline; cursor persists across frames).
    /// Each drained sample passes the stereo DC-block HPF
    /// (`nerust_sound_filter::IirFilter`, rebuilt on device-rate change).
    pub fn drain_resampled(&mut self, rate: u32) -> Vec<StereoSample> {
        if self.mix_buffer.is_empty() || rate == 0 {
            return Vec::new();
        }
        if self.output_hpf_rate != rate {
            self.output_hpf_l = IirFilter::get_highpass_filter(rate as f32, OUTPUT_HPF_CUTOFF_HZ);
            self.output_hpf_r = IirFilter::get_highpass_filter(rate as f32, OUTPUT_HPF_CUTOFF_HZ);
            self.output_hpf_rate = rate;
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
        for sample in &mut out {
            sample.left = self.output_hpf_l.step(sample.left);
            sample.right = self.output_hpf_r.step(sample.right);
        }
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
    /// Writes while master enable (SOUNDCNT_X bit 7) is off are dropped:
    /// HW leaves the FIFO empty so a later timer overflow still requests
    /// its DMA channel (alyosha fifo t002 observes DMA fire with the
    /// FIFO supposedly loaded).
    pub fn push_fifo(&mut self, fifo_b: bool, value: u32, width: u8) {
        if self.soundcnt_x & 0x80 == 0 {
            return;
        }
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
        // Write-time R/W masks (GBATEK R/W maps).
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

#[cfg(test)]
mod tests {
    use super::*;

    fn dc_apu(frames: usize, level: f32) -> GbaApu {
        let mut apu = GbaApu::new();
        apu.mix_buffer = vec![(level, level); frames];
        apu
    }

    #[test]
    fn output_hpf_blocks_dc_but_passes_edges() {
        let mut apu = dc_apu(4800, 0.5);
        let out = apu.drain_resampled(48_000);
        assert!(!out.is_empty());
        // Fast edges pass near unity through the first-order HPF...
        let peak = out.iter().take(10).map(|s| s.left).fold(0.0f32, f32::max);
        assert!(peak > 0.4, "HPF should pass the leading edge, got {peak}");
        // ...while sustained DC converges to silence (tau ~= 8ms at 20Hz).
        let tail = out[out.len() - 1];
        assert!(
            tail.left.abs() < 0.01 && tail.right.abs() < 0.01,
            "DC should converge to 0, got ({}, {})",
            tail.left,
            tail.right
        );
    }

    #[test]
    fn output_hpf_rebuilds_on_rate_change() {
        let mut apu = dc_apu(1024, 0.25);
        let first = apu.drain_resampled(48_000);
        assert!(!first.is_empty());
        assert_eq!(apu.output_hpf_rate, 48_000);
        apu.mix_buffer = vec![(0.25, -0.25); 1024];
        let second = apu.drain_resampled(44_100);
        assert!(!second.is_empty());
        assert_eq!(apu.output_hpf_rate, 44_100);
        assert!(
            second
                .iter()
                .all(|s| s.left.is_finite() && s.right.is_finite())
        );
    }

    #[test]
    fn output_hpf_state_clears_on_sound_reset() {
        let mut apu = dc_apu(4800, 0.5);
        let _ = apu.drain_resampled(48_000);
        apu.reset_sound();
        // Fresh filter memory: the next leading edge passes near unity again
        // instead of continuing from the converged (silent) state.
        apu.mix_buffer = vec![(0.5, 0.5); 64];
        let out = apu.drain_resampled(48_000);
        let peak = out.iter().map(|s| s.left).fold(0.0f32, f32::max);
        assert!(peak > 0.4, "reset should clear HPF memory, got {peak}");
    }

    #[test]
    fn apu_state_round_trips_mid_note() {
        let mut apu = GbaApu::new();
        apu.write_soundcnt_x(0x80);
        // Ch1: sweep off, duty 2, envelope up, freq with trigger.
        apu.write_sound1cnt_lo(0x0040);
        apu.write_sound1cnt_hi(0x81F3);
        apu.write_sound1cnt_x(0x8385);
        // FIFO A gets two samples (non-empty FIFO path).
        apu.fifo_a.extend([0x10, 0x20]);
        apu.dac_a = 0x10;
        // A driver voice is mid-note.
        apu.driver_voices[0] = DriverVoice {
            started: true,
            pos: 12.5,
            env: 200.0,
        };
        // Run long enough for the envelope and sequencer to advance and
        // the grid buffer to fill.
        for _ in 0..4000 {
            apu.tick();
        }
        assert!(!apu.mix_buffer.is_empty());
        // A full grid buffer is mid-frame content: export must refuse.
        assert!(apu.export_state().is_err());
        // Draining leaves only the interpolation tail: export succeeds.
        let _ = apu.drain_resampled(48_000);
        assert!(apu.mix_buffer.len() <= 2);

        let state = apu.export_state().unwrap();
        state.validate().unwrap();
        let bytes = rmp_serde::to_vec_named(&state).unwrap();
        let decoded: GbaApuState = rmp_serde::from_slice(&bytes).unwrap();
        decoded.validate().unwrap();
        let mut restored = GbaApu::new();
        restored.import_state(decoded).unwrap();
        let again = rmp_serde::to_vec_named(&restored.export_state().unwrap()).unwrap();
        assert_eq!(bytes, again);
        assert!(restored.mix_buffer.is_empty());
        assert_eq!(restored.output_hpf_rate, OUTPUT_HPF_DEFAULT_RATE);

        let mut bad = restored.export_state().unwrap();
        bad.seq_step = 8;
        assert!(bad.validate().is_err());
        let mut bad = restored.export_state().unwrap();
        bad.fifo_a = vec![0; 33];
        assert!(bad.validate().is_err());
    }
}
