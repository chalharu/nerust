/// APU register file, NR52 power control and channel length/trigger logic.
///
/// Channel synthesis (audio output) is still a stub, but the timing behaviour
/// required by the retrio sound tests is modelled:
/// - write/read of NR10-NR51 with the hardware read masks (unused bits = 1)
/// - wave RAM (FF30-FF3F) read/write
/// - NR52 power bit 7; powering off resets every register and ignores writes
/// - the frame sequencer (DIV-APU): length clock at 256 Hz, plus the "enable
///   length in the first half of a length period" glitch on NRx4 writes
/// - length timer per channel: loaded on NRx1/NR31 write, counted down, and
///   reloaded with the maximum when triggered at zero
/// - DAC gating: a disabled DAC turns a channel off and blocks trigger enables
/// - NR52 bits 0-3 reflect channel activity
#[derive(Debug, Clone, Default)]
pub struct GbcApu {
    /// Stored values for FF10-FF26 (unused bits cleared; masks applied on read).
    regs: [u8; 0x17],
    /// Wave RAM FF30-FF3F.
    wave_ram: [u8; 16],
    /// NR52 bit 7: APU power. When off, registers read as their masks and
    /// writes are ignored.
    powered: bool,
    /// T-cycles accumulated since the last DIV-APU event.
    apu_dot_clock: u32,
    /// DIV-APU event counter (increments every 8192 T-cycles). The length
    /// clock fires when it is odd (every second event, i.e. 256 Hz).
    div_divider: u8,
    /// Channel state for CH1-CH4.
    channels: [Channel; 4],
    /// CH1 frequency sweep state.
    sweep: Sweep,
    /// Whether the hardware is a CGB/AGB (affects the power-off reset).
    cgb: bool,
}

/// CH1 frequency sweep (NR10 / NR14 bit 7).
#[derive(Debug, Clone, Default)]
struct Sweep {
    enabled: bool,
    /// Shadow frequency register.
    shadow: u16,
    /// `shadow >> shift`, the value added/subtracted each sweep step.
    addend: u16,
    /// Sweep period countdown (inverted: starts at `period ^ 7`, wraps at 7).
    countdown: u8,
    period: u8,
    shift: u8,
    negate: bool,
    /// Current CH1 frequency (NR13 | (NR14 & 7) << 8).
    freq: u16,
}

/// Per-channel state relevant to the length timer and NR52 status.
#[derive(Debug, Clone, Default)]
struct Channel {
    /// Remaining length timer count (1..=max; 0 once it ran out).
    length: u16,
    /// Whether the length timer is enabled (NRx4 bit 6).
    length_enabled: bool,
    /// NR52 status bit: channel is producing sound.
    active: bool,
}

impl Channel {
    /// Maximum length counter value: 256 for the wave channel, 64 otherwise.
    fn max_len(ch: usize) -> u16 {
        if ch == 2 {
            256
        } else {
            64
        }
    }

    fn clock_length(&mut self, _ch: usize) {
        if self.length_enabled && self.length > 0 {
            self.length -= 1;
            if self.length == 0 {
                self.active = false;
            }
        }
    }
}

/// Read mask for each register FF10-FF26: bits that always read as 1.
const MASKS: [u8; 0x17] = [
    0x80, 0x3F, 0x00, 0xFF, 0xBF, // FF10-FF14 (NR10-NR14)
    0xFF, 0x3F, 0x00, 0xFF, 0xBF, // FF15-FF19 (NR2x)
    0x7F, 0xFF, 0x9F, 0xFF, 0xBF, // FF1A-FF1E (NR3x)
    0xFF, 0xFF, 0x00, 0x00, 0xBF, // FF1F-FF23 (NR4x)
    0x00, 0x00, 0x70,             // FF24-FF26 (NR50, NR51, NR52)
];

impl GbcApu {
    pub fn step(&mut self, cycles: u32) {
        if !self.powered {
            // The frame sequencer only advances while the APU is powered.
            return;
        }
        self.apu_dot_clock += cycles;
        while self.apu_dot_clock >= 8192 {
            self.apu_dot_clock -= 8192;
            self.div_divider = self.div_divider.wrapping_add(1);
            if self.div_divider & 1 == 1 {
                // Length clock (256 Hz).
                for ch in 0..4 {
                    self.channels[ch].clock_length(ch);
                }
            }
            // CH1 frequency sweep (128 Hz).
            if self.div_divider & 3 == 3 {
                self.sweep_step();
            }
            // Volume envelope (64 Hz) does not affect the register-visible
            // state modelled here.
        }
    }

    /// Advance the CH1 frequency sweep by one 128 Hz tick.
    fn sweep_step(&mut self) {
        // With a zero period the sweep never ticks periodically; only the
        // trigger performs a calculation.
        if !self.sweep.enabled || self.sweep.period == 0 {
            return;
        }
        self.sweep.countdown = (self.sweep.countdown + 1) & 7;
        if self.sweep.countdown != 7 {
            return;
        }
        let addend = self.sweep.addend;
        let new_freq = if self.sweep.negate {
            self.sweep.shadow.wrapping_sub(addend)
        } else {
            self.sweep.shadow + addend
        };
        // APU bug: the overflow check compares the new frequency plus its own
        // shifted addend (the delta is effectively added twice).
        let next_addend = new_freq >> self.sweep.shift;
        let overflow = if self.sweep.negate {
            new_freq < next_addend
        } else {
            new_freq + next_addend > 0x7FF
        };
        if overflow {
            self.channels[0].active = false;
        } else {
            self.sweep.shadow = new_freq;
            self.sweep.freq = new_freq;
            self.sweep.addend = next_addend;
        }
        self.sweep.countdown = (self.sweep.period ^ 7) & 7;
    }

    pub fn flush_samples(&mut self) -> Vec<f32> {
        Vec::new()
    }

    /// Set whether the hardware is a CGB (affects the power-off reset quirk:
    /// on DMG the NRx1 length registers and counters survive a power cycle).
    pub fn set_cgb(&mut self, cgb: bool) {
        self.cgb = cgb;
    }

    /// Whether the DAC of a channel is enabled (square/noise: NRx2 & $F8;
    /// wave: NR30 bit 7).
    fn dac_enabled(&self, ch: usize) -> bool {
        let idx = match ch {
            0 => 2,  // NR12
            1 => 7,  // NR22
            2 => 10, // NR30
            3 => 17, // NR42
            _ => return false,
        };
        if ch == 2 {
            self.regs[idx] & 0x80 != 0
        } else {
            self.regs[idx] & 0xF8 != 0
        }
    }

    /// Write to NRx1 (length): loads the length counter immediately.
    fn write_length(&mut self, ch: usize, value: u8) {
        let length = if ch == 2 {
            256 - u16::from(value)
        } else {
            64 - u16::from(value & 0x3F)
        };
        self.channels[ch].length = length;
    }

    /// Write to NRx2 (volume/DAC for square/noise channels).
    fn write_dac(&mut self, ch: usize, value: u8) {
        if value & 0xF8 == 0 {
            self.channels[ch].active = false;
        }
    }

    /// Write to NR30 (wave channel DAC).
    fn write_wave_dac(&mut self, value: u8) {
        if value & 0x80 == 0 {
            self.channels[2].active = false;
        }
    }

    /// Write to NRx4: length enable, trigger and the enable glitch.
    fn write_nrx4(&mut self, ch: usize, value: u8) {
        let was_active = self.channels[ch].active;
        if value & 0x80 != 0 {
            // Trigger.
            if self.channels[ch].length == 0 {
                self.channels[ch].length = Channel::max_len(ch);
                self.channels[ch].length_enabled = false;
            }
            if self.dac_enabled(ch) && !was_active {
                self.channels[ch].active = true;
            }
            if ch == 0 {
                // Reload the frequency sweep shadow registers. NR13 is
                // write-only (mask 0xFF) so its value lives in sweep.freq.
                self.sweep.freq =
                    (self.sweep.freq & 0x00FF) | ((value as u16 & 0x07) << 8);
                self.sweep.period = (self.regs[0] >> 4) & 7;
                self.sweep.shift = self.regs[0] & 7;
                self.sweep.negate = self.regs[0] & 8 != 0;
                self.sweep.shadow = self.sweep.freq;
                self.sweep.addend = self.sweep.freq >> self.sweep.shift;
                self.sweep.countdown = (self.sweep.period ^ 7) & 7;
                self.sweep.enabled = (self.sweep.period | self.sweep.shift) != 0;
                // APU bug: if the shift is non-zero, the overflow check also
                // occurs on trigger.
                if self.sweep.shift != 0 {
                    let overflow = if self.sweep.negate {
                        self.sweep.freq < self.sweep.addend
                    } else {
                        self.sweep.freq + self.sweep.addend > 0x7FF
                    };
                    if overflow {
                        self.channels[0].active = false;
                    }
                }
            }
        }
        // APU glitch: enabling the length timer while the DIV-divider's LSB
        // is 1 (first half of the length period) ticks the length once.
        if value & 0x40 != 0
            && !self.channels[ch].length_enabled
            && self.div_divider & 1 == 1
            && self.channels[ch].length != 0
        {
            self.channels[ch].length -= 1;
            if self.channels[ch].length == 0 {
                if value & 0x80 != 0 {
                    self.channels[ch].length = Channel::max_len(ch) - 1;
                } else {
                    self.channels[ch].active = false;
                }
            }
        }
        self.channels[ch].length_enabled = value & 0x40 != 0;
    }

    pub fn read_register(&self, addr: u16) -> u8 {
        match addr {
            0xFF10..=0xFF26 => {
                let idx = (addr - 0xFF10) as usize;
                if addr == 0xFF26 {
                    // NR52: bits 4-6 always 1, bit 7 = power, bits 0-3 =
                    // channel activity.
                    let mut v = 0x70 | (if self.powered { 0x80 } else { 0 });
                    for ch in 0..4 {
                        if self.channels[ch].active {
                            v |= 1 << ch;
                        }
                    }
                    v
                } else if !self.powered {
                    // Power off: registers read as their mask (values reset).
                    MASKS[idx]
                } else {
                    self.regs[idx] | MASKS[idx]
                }
            }
            0xFF27..=0xFF2F => 0xFF,
            0xFF30..=0xFF3F => self.wave_ram[(addr - 0xFF30) as usize],
            _ => 0xFF,
        }
    }

    pub fn write_register(&mut self, addr: u16, value: u8) {
        match addr {
            0xFF10..=0xFF26 => {
                let idx = (addr - 0xFF10) as usize;
                if addr == 0xFF26 {
                    // NR52: bit 7 controls power. Turning power off resets
                    // all registers and ignores further writes until power is
                    // restored.
                    let powered = value & 0x80 != 0;
                    if self.powered && !powered {
                        // Powering off clears every register and stops all
                        // channels. On DMG the NRx1 length registers and the
                        // length counters survive the power cycle.
                        let len_regs = [self.regs[1], self.regs[6], self.regs[11], self.regs[16]];
                        let lengths: [u16; 4] =
                            std::array::from_fn(|i| self.channels[i].length);
                        self.regs.fill(0);
                        self.channels.fill(Channel::default());
                        self.sweep = Sweep::default();
                        self.div_divider = 0;
                        self.apu_dot_clock = 0;
                        if !self.cgb {
                            for (i, reg_idx) in [1usize, 6, 11, 16].into_iter().enumerate() {
                                self.regs[reg_idx] = len_regs[i];
                                self.channels[i].length = lengths[i];
                            }
                        }
                    } else if !self.powered && powered {
                        self.div_divider = 1;
                        self.apu_dot_clock = 0;
                    }
                    self.powered = powered;
                    self.regs[idx] = value & 0x8F;
                } else if self.powered {
                    self.regs[idx] = value & !MASKS[idx];
                    match idx {
                        0 => {
                            // NR10: sweep config.
                            self.sweep.period = (self.regs[0] >> 4) & 7;
                            self.sweep.shift = self.regs[0] & 7;
                            self.sweep.negate = self.regs[0] & 8 != 0;
                        }
                        3 => {
                            // NR13: CH1 frequency low byte.
                            self.sweep.freq = (self.sweep.freq & 0x700) | value as u16;
                        }
                        1 | 6 | 16 => self.write_length(idx / 5, value), // NR11/NR21/NR41
                        11 => self.write_length(2, value),               // NR31
                        2 | 7 => self.write_dac(idx / 5, value),         // NR12/NR22
                        17 => self.write_dac(3, value),                  // NR42
                        10 => self.write_wave_dac(value),                // NR30
                        4 | 9 | 14 | 19 => self.write_nrx4(idx / 5, value), // NR14/NR24/NR34/NR44
                        _ => {}
                    }
                } else if !self.cgb {
                    // On DMG the NRx1 length registers remain writable while
                    // the APU is off; every other register is read-only.
                    match idx {
                        1 | 6 | 16 => {
                            self.regs[idx] = value & !MASKS[idx];
                            self.write_length(idx / 5, value);
                        }
                        11 => {
                            self.regs[idx] = value & !MASKS[idx];
                            self.write_length(2, value);
                        }
                        _ => {}
                    }
                }
                // else: power off, writes ignored.
            }
            0xFF30..=0xFF3F => {
                self.wave_ram[(addr - 0xFF30) as usize] = value;
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn register_write_read_with_masks() {
        let mut apu = GbcApu::default();
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
        let mut apu = GbcApu::default();
        apu.write_register(0xFF26, 0x00);
        assert_eq!(apu.read_register(0xFF26), 0x70);
        apu.write_register(0xFF26, 0xFF);
        assert_eq!(apu.read_register(0xFF26), 0xF0);
    }

    #[test]
    fn power_off_resets_and_ignores_writes() {
        let mut apu = GbcApu::default();
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
        let mut apu = GbcApu::default();
        apu.write_register(0xFF30, 0x37);
        apu.write_register(0xFF31, 0xAB);
        assert_eq!(apu.read_register(0xFF30), 0x37);
        assert_eq!(apu.read_register(0xFF31), 0xAB);
    }
}

