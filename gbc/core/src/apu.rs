/// APU register file and NR52 power control.
///
/// The four sound channels are not yet implemented (audio output is a stub),
/// but the register behaviour required by the retrio sound tests is modelled:
/// - write/read of NR10-NR51 with the hardware read masks (unused bits = 1)
/// - wave RAM (FF30-FF3F) read/write
/// - NR52 power bit 7; powering off resets every register and ignores writes
#[derive(Debug, Clone, Default)]
pub struct GbcApu {
    /// Stored values for FF10-FF26 (unused bits cleared; masks applied on read).
    regs: [u8; 0x17],
    /// Wave RAM FF30-FF3F.
    wave_ram: [u8; 16],
    /// NR52 bit 7: APU power. When off, registers read as their masks and
    /// writes are ignored.
    powered: bool,
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
    pub fn step(&mut self, _cycles: u32) {
        // Channel synthesis not implemented yet.
    }

    pub fn flush_samples(&mut self) -> Vec<f32> {
        Vec::new()
    }

    pub fn read_register(&self, addr: u16) -> u8 {
        match addr {
            0xFF10..=0xFF26 => {
                let idx = (addr - 0xFF10) as usize;
                if addr == 0xFF26 {
                    // NR52: bits 4-6 always 1, bit 7 = power, bits 0-3 =
                    // channel activity (no channels active yet).
                    0x70 | (if self.powered { 0x80 } else { 0 })
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
                        self.regs.fill(0);
                    }
                    self.powered = powered;
                    self.regs[idx] = value & 0x8F;
                } else if self.powered {
                    self.regs[idx] = value & !MASKS[idx];
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

