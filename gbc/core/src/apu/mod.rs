pub(crate) mod envelope;
pub(crate) mod high_pass;
pub(crate) mod length_counter;
pub(crate) mod mixer;
pub(crate) mod noise;
pub(crate) mod square1;
pub(crate) mod square2;
pub(crate) mod timer;
pub(crate) mod wave;

use high_pass::HighPassFilter;
use mixer::Mixer;
use noise::Noise;
use square1::Square1;
use square2::Square2;
use wave::Wave;

/// Master clock frequency (1x speed).
const MASTER_CLOCK: u32 = 4_194_304;
/// Output sample rate.
const SAMPLE_RATE: u32 = 44_100;

/// Read mask for each register FF10-FF26: bits that always read as 1.
const MASKS: [u8; 0x17] = [
    0x80, 0x3F, 0x00, 0xFF, 0xBF, // FF10-FF14 (NR10-NR14)
    0xFF, 0x3F, 0x00, 0xFF, 0xBF, // FF15-FF19 (NR2x)
    0x7F, 0xFF, 0x9F, 0xFF, 0xBF, // FF1A-FF1E (NR3x)
    0xFF, 0xFF, 0x00, 0x00, 0xBF, // FF1F-FF23 (NR4x)
    0x00, 0x00, 0x70, // FF24-FF26 (NR50, NR51, NR52)
];

/// GBC Audio Processing Unit.
#[derive(Debug, Clone)]
pub struct GbcApu {
    /// Stored values for FF10-FF26.
    regs: [u8; 0x17],
    /// Wave RAM FF30-FF3F.
    wave_ram: [u8; 16],
    /// NR52 bit 7: APU power.
    powered: bool,
    /// Whether the hardware is a CGB.
    cgb: bool,

    // Channels
    ch1: Square1,
    ch2: Square2,
    ch3: Wave,
    ch4: Noise,

    // Control
    mixer: Mixer,
    hpf_left: HighPassFilter,
    hpf_right: HighPassFilter,

    // DIV-APU counter
    div_apu_counter: u32,
    div_divider: u8,
    /// Dot counter for Square/Wave channel timer prescaler.
    /// Square/Wave channels are clocked at 1,048,576 Hz (master/4).
    dot_counter: u32,

    // Downsampling
    sample_accumulator: u32,
    output_buffer: Vec<f32>,
}

impl GbcApu {
    /// Create a new APU instance.
    pub fn new() -> Self {
        Self {
            regs: [0; 0x17],
            wave_ram: [0; 16],
            powered: false,
            cgb: false,

            ch1: Square1::new(),
            ch2: Square2::new(),
            ch3: Wave::new(),
            ch4: Noise::new(),

            mixer: Mixer::new(),
            hpf_left: HighPassFilter::new(false, SAMPLE_RATE),
            hpf_right: HighPassFilter::new(false, SAMPLE_RATE),

            div_apu_counter: 0,
            div_divider: 0,
            dot_counter: 0,

            sample_accumulator: 0,
            output_buffer: Vec::new(),
        }
    }

    /// Step the APU by the given number of T-cycles.
    pub fn step(&mut self, cycles: u32) {
        if !self.powered {
            return;
        }

        for _ in 0..cycles {
            // 1. DIV-APU update (512 Hz)
            self.div_apu_counter += 1;
            if self.div_apu_counter >= 8192 {
                self.div_apu_counter -= 8192;
                self.div_divider = self.div_divider.wrapping_add(1);

                // Length clock (256 Hz)
                if self.div_divider & 1 == 1 {
                    self.clock_length();
                }
                // Sweep clock (128 Hz)
                if self.div_divider & 3 == 3 {
                    self.clock_sweep();
                }
                // Envelope clock (64 Hz)
                if self.div_divider & 7 == 7 {
                    self.clock_envelope();
                }
            }

            // 2. Channel timers
            // Square channels: clocked at 1,048,576 Hz (master/4)
            // Pan Docs: "The pulse channels' period dividers are clocked
            // at 1048576 Hz, once per four dots"
            self.dot_counter += 1;
            // Square channels: clocked at 1,048,576 Hz (master/4)
            // Pan Docs: "The pulse channels' period dividers are clocked
            // at 1048576 Hz, once per four dots"
            if self.dot_counter.is_multiple_of(4) {
                self.ch1.step();
                self.ch2.step();
            }
            // Wave channel: clocked at 2,097,152 Hz (master/2)
            // Pan Docs: "The wave channel's period divider is clocked
            // at 2097152 Hz, once per two dots"
            if self.dot_counter.is_multiple_of(2) {
                self.ch3.step();
            }
            // Noise channel: clocked at 1,048,576 Hz (master/4)
            // DIVISOR_TABLE values are calibrated for this clock rate
            if self.dot_counter.is_multiple_of(4) {
                self.ch4.step();
            }

            // 3. Sample generation (44,100 Hz)
            self.sample_accumulator += SAMPLE_RATE;
            if self.sample_accumulator >= MASTER_CLOCK {
                self.sample_accumulator -= MASTER_CLOCK;
                let sample = self.generate_sample();
                self.output_buffer.push(sample);
            }
        }
    }

    /// Clock length counters for all channels.
    fn clock_length(&mut self) {
        if self.ch1.length.clock() {
            self.ch1.active = false;
        }
        if self.ch2.length.clock() {
            self.ch2.active = false;
        }
        if self.ch3.length.clock() {
            self.ch3.active = false;
        }
        if self.ch4.length.clock() {
            self.ch4.active = false;
        }
    }

    /// Clock frequency sweep for CH1.
    fn clock_sweep(&mut self) {
        self.ch1.clock_sweep();
    }

    /// Clock volume envelopes for CH1, CH2, CH4.
    fn clock_envelope(&mut self) {
        self.ch1.envelope.clock();
        self.ch2.envelope.clock();
        self.ch4.envelope.clock();
    }

    /// Check if the next DIV-APU tick would clock the envelope.
    /// Pan Docs: "If a channel is triggered when the DIV-APU next step
    /// will clock the volume envelope, the envelope's timer is reloaded
    /// with one greater than it would have been."
    fn should_envelope_extra_tick(&self) -> bool {
        // Envelope clock occurs when div_divider & 7 == 7
        // The next tick will clock envelope if current state + 1 & 7 == 7
        self.div_divider.wrapping_add(1) & 7 == 7
    }

    /// Generate one audio sample at 44,100 Hz.
    fn generate_sample(&mut self) -> f32 {
        let ch1 = self.ch1.output();
        let ch2 = self.ch2.output();
        let ch3 = self.ch3.output();
        let ch4 = self.ch4.output();

        let (left, right) = self.mixer.mix(ch1, ch2, ch3, ch4);

        // HPF
        let dacs_enabled = self.ch1.dac_enabled
            || self.ch2.dac_enabled
            || self.ch3.dac_enabled
            || self.ch4.dac_enabled;
        let left = self.hpf_left.step(left as f64, dacs_enabled) as f32;
        let right = self.hpf_right.step(right as f64, dacs_enabled) as f32;

        // Mono output (average of left and right)
        (left + right) / 2.0
    }

    /// Flush the output buffer (called once per frame).
    pub fn flush_samples(&mut self) -> Vec<f32> {
        std::mem::take(&mut self.output_buffer)
    }

    /// Reset the DIV-APU frame sequencer counter.
    /// Called when the CPU writes to the DIV register ($FF04).
    /// In real hardware, the frame sequencer shares the same 16-bit counter
    /// as the DIV register, so writing to DIV resets both.
    /// If bit 4 of the old DIV value was 1, a falling edge occurs and
    /// the DIV-APU counter is incremented before being reset.
    pub fn reset_div_apu(&mut self, div_bit4_was_set: bool) {
        if div_bit4_was_set {
            // Falling edge of DIV bit 4: increment frame sequencer
            self.div_divider = self.div_divider.wrapping_add(1);
        }
        self.div_apu_counter = 0;
    }

    /// Set whether the hardware is a CGB.
    pub fn set_cgb(&mut self, cgb: bool) {
        self.cgb = cgb;
        self.hpf_left = HighPassFilter::new(cgb, SAMPLE_RATE);
        self.hpf_right = HighPassFilter::new(cgb, SAMPLE_RATE);
    }

    /// Apply the post-boot register values.
    pub fn set_post_boot_state(&mut self) {
        self.powered = true;
        self.regs = [
            0x00, 0x80, 0xF3, 0x00, 0x00, // NR10-NR14
            0x00, 0x00, 0x00, 0x00, 0x00, // NR2x
            0x00, 0x00, 0x00, 0x00, 0x00, // NR3x
            0x00, 0x00, 0x00, 0x00, 0x00, // NR4x
            0x77, 0xF3, 0x81, // NR50, NR51, NR52
        ];
        self.ch1.active = true;
    }

    /// Read a register at the given address.
    pub fn read_register(&self, addr: u16) -> u8 {
        match addr {
            0xFF10..=0xFF26 => {
                let idx = (addr - 0xFF10) as usize;
                if addr == 0xFF26 {
                    return self.read_nr52();
                }
                if !self.powered {
                    return MASKS[idx];
                }
                self.regs[idx] | MASKS[idx]
            }
            0xFF27..=0xFF2F => 0xFF,
            0xFF30..=0xFF3F => {
                if self.ch3.active && self.powered {
                    if self.cgb {
                        // CGB: return byte at current playback position
                        self.ch3.wave_ram[self.ch3.position as usize / 2]
                    } else {
                        // DMG: return $FF when CH3 is active
                        // Pan Docs: "On monochrome consoles, wave RAM can
                        // only be accessed on the same cycle that CH3 does.
                        // Otherwise, reads return $FF"
                        0xFF
                    }
                } else {
                    self.wave_ram[(addr - 0xFF30) as usize]
                }
            }
            0xFF76 if self.cgb => {
                // PCM12 (CGB only): CH1 low nibble, CH2 high nibble
                (self.ch2.output() << 4) | self.ch1.output()
            }
            0xFF76 => 0xFF,
            0xFF77 if self.cgb => {
                // PCM34 (CGB only): CH3 low nibble, CH4 high nibble
                (self.ch4.output() << 4) | self.ch3.output()
            }
            0xFF77 => 0xFF,
            _ => 0xFF,
        }
    }

    /// Write to a register at the given address.
    pub fn write_register(&mut self, addr: u16, value: u8) {
        match addr {
            0xFF10..=0xFF26 => {
                let idx = (addr - 0xFF10) as usize;
                if addr == 0xFF26 {
                    self.handle_power_change(value);
                    self.regs[idx] = value & 0x8F;
                } else if self.powered {
                    self.regs[idx] = value & !MASKS[idx];
                    self.dispatch_register_write(idx, value);
                } else if !self.cgb {
                    self.handle_dmg_off_write(idx, value);
                }
            }
            0xFF30..=0xFF3F => {
                if self.ch3.active && self.powered {
                    if self.cgb {
                        // CGB: write to current playback position
                        let pos = self.ch3.position as usize / 2;
                        self.wave_ram[pos] = value;
                        self.ch3.wave_ram[pos] = value;
                    } else {
                        // DMG: writes are ignored when CH3 is active
                        // Pan Docs: "On monochrome consoles, wave RAM can
                        // only be accessed on the same cycle that CH3 does.
                        // Otherwise, ... writes are ignored"
                    }
                } else {
                    self.wave_ram[(addr - 0xFF30) as usize] = value;
                    self.ch3.wave_ram[(addr - 0xFF30) as usize] = value;
                }
            }
            _ => {}
        }
    }

    /// Read NR52 register.
    fn read_nr52(&self) -> u8 {
        let mut v = 0x70 | if self.powered { 0x80 } else { 0 };
        if self.ch1.active {
            v |= 0x01;
        }
        if self.ch2.active {
            v |= 0x02;
        }
        if self.ch3.active {
            v |= 0x04;
        }
        if self.ch4.active {
            v |= 0x08;
        }
        v
    }

    /// Handle NR52 power change.
    fn handle_power_change(&mut self, value: u8) {
        let powered = value & 0x80 != 0;
        if self.powered && !powered {
            // Powering off clears registers and stops channels.
            // Save internal length counter values before clearing.
            let lengths = [
                self.ch1.length.counter(),
                self.ch2.length.counter(),
                self.ch3.length.counter(),
                self.ch4.length.counter(),
            ];
            self.regs.fill(0);
            self.ch1 = Square1::new();
            self.ch2 = Square2::new();
            self.ch3 = Wave::new();
            self.ch4 = Noise::new();
            self.mixer = Mixer::new();
            self.div_divider = 0;
            self.div_apu_counter = 0;
            self.dot_counter = 0;
            // On DMG, internal length counters survive power cycle
            // (but register values are cleared like all other registers)
            if !self.cgb {
                self.ch1.length.set_counter(lengths[0]);
                self.ch2.length.set_counter(lengths[1]);
                self.ch3.length.set_counter(lengths[2]);
                self.ch4.length.set_counter(lengths[3]);
            }
            // Clear wave RAM buffer
            self.ch3.clear_buffer();
        } else if !self.powered && powered {
            self.div_divider = 0;
            self.div_apu_counter = 0;
        }
        self.powered = powered;
    }

    /// Dispatch register write to appropriate channel.
    fn dispatch_register_write(&mut self, idx: usize, value: u8) {
        // Pan Docs: Length glitch occurs when writing to NRx4 when the
        // DIV-APU next step is one that doesn't clock the length timer.
        // This means the DIV LSB is 1 (next step won't clock length).
        let next_div_lsb = self.div_divider & 1 == 1;
        // Pan Docs: If a channel is triggered when the DIV-APU next step
        // will clock the volume envelope, the envelope's timer is reloaded
        // with one greater than it would have been.
        let envelope_extra_tick = self.should_envelope_extra_tick();
        match idx {
            // NR10
            0 => self.ch1.write_nr10(value),
            // NR11
            1 => self.ch1.write_nr11(value),
            // NR12
            2 => self.ch1.write_nr12(value),
            // NR13
            3 => self.ch1.write_nr13(value),
            // NR14
            4 => self
                .ch1
                .write_nr14(value, next_div_lsb, envelope_extra_tick),
            // NR21
            6 => self.ch2.write_nr21(value),
            // NR22
            7 => self.ch2.write_nr22(value),
            // NR23
            8 => self.ch2.write_nr23(value),
            // NR24
            9 => self
                .ch2
                .write_nr24(value, next_div_lsb, envelope_extra_tick),
            // NR30
            10 => self.ch3.write_nr30(value),
            // NR31
            11 => self.ch3.write_nr31(value),
            // NR32
            12 => self.ch3.write_nr32(value),
            // NR33
            13 => self.ch3.write_nr33(value),
            // NR34
            14 => self.ch3.write_nr34(value, next_div_lsb),
            // NR41
            16 => self.ch4.write_nr41(value),
            // NR42
            17 => self.ch4.write_nr42(value),
            // NR43
            18 => self.ch4.write_nr43(value),
            // NR44
            19 => self
                .ch4
                .write_nr44(value, next_div_lsb, envelope_extra_tick),
            // NR50
            20 => self.mixer.write_nr50(value),
            // NR51
            21 => self.mixer.write_nr51(value),
            _ => {}
        }
    }

    /// Handle DMG off-write behavior (NRx1 length registers remain writable).
    /// On DMG, writing to NRx1 while the APU is off only updates the
    /// internal length counter, not the register value.
    fn handle_dmg_off_write(&mut self, idx: usize, value: u8) {
        match idx {
            1 => {
                self.ch1.length.load(value & 0x3F);
            }
            6 => {
                self.ch2.length.load(value & 0x3F);
            }
            11 => {
                self.ch3.length.load(value);
            }
            16 => {
                self.ch4.length.load(value);
            }
            _ => {}
        }
    }
}

impl Default for GbcApu {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn register_write_read_with_masks() {
        let mut apu = GbcApu::new();
        apu.write_register(0xFF26, 0x80); // power on
        for i in 0..0x17u16 {
            let addr = 0xFF10 + i;
            if addr == 0xFF26 {
                continue;
            }
            let d = 0x55u8;
            apu.write_register(addr, d);
            let read = apu.read_register(addr);
            assert_eq!(read, MASKS[i as usize] | d, "FF{:02X}", 0x10 + i);
        }
    }

    #[test]
    fn nr52_power_control() {
        let mut apu = GbcApu::new();
        apu.write_register(0xFF26, 0x00);
        assert_eq!(apu.read_register(0xFF26), 0x70);
        apu.write_register(0xFF26, 0xFF);
        assert_eq!(apu.read_register(0xFF26), 0xF0);
    }

    #[test]
    fn power_off_resets_and_ignores_writes() {
        let mut apu = GbcApu::new();
        apu.write_register(0xFF26, 0x80);
        apu.write_register(0xFF10, 0xAA);
        assert_eq!(apu.read_register(0xFF10), 0xAA | MASKS[0]);
        apu.write_register(0xFF26, 0x00); // power off
        assert_eq!(apu.read_register(0xFF10), MASKS[0]);
        apu.write_register(0xFF10, 0x55); // ignored
        assert_eq!(apu.read_register(0xFF10), MASKS[0]);
        apu.write_register(0xFF26, 0x80); // power on
        assert_eq!(apu.read_register(0xFF10), MASKS[0]);
    }

    #[test]
    fn wave_ram_read_write() {
        let mut apu = GbcApu::new();
        apu.write_register(0xFF30, 0x37);
        apu.write_register(0xFF31, 0xAB);
        assert_eq!(apu.read_register(0xFF30), 0x37);
        assert_eq!(apu.read_register(0xFF31), 0xAB);
    }

    #[test]
    fn audio_output_not_empty() {
        let mut apu = GbcApu::new();
        apu.write_register(0xFF26, 0x80); // power on
        // Enable CH1 with max volume
        apu.write_register(0xFF12, 0xF0); // NR12: volume 15, no sweep
        apu.write_register(0xFF11, 0x80); // NR11: duty 2, length 0
        apu.write_register(0xFF14, 0x80); // NR14: trigger

        // Step for one frame (70224 T-cycles for NTSC)
        apu.step(70224);

        let samples = apu.flush_samples();
        assert!(!samples.is_empty());
    }
}
