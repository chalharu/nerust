/// Boktai solar sensor (ramp ADC) after NBA `HW/GamePak/GPIO/SolarSensor`,
/// cross-checked against GBAHawk `Mappers.h`, GBATEK "Game Pak Solar Sensor".
///
/// Pins: 0 = CLK, 1 = RST, 3 = FLG. RST high resets the counter; each
/// CLK falling edge (with RST and CS low) increments it; FLG reads high
/// once the counter reaches the light level. Games count clocks to FLG
/// (`00h` = blinding .. `E8h` = dark per GBATEK).
#[derive(Debug, Clone, Copy)]
pub struct Solar {
    counter: u8,
    level: u8,
    light: u8,
}

impl Default for Solar {
    fn default() -> Self {
        Self {
            counter: 0,
            // Mid-scale daylight; games calibrate against it.
            level: 0x60,
            light: 0x60,
        }
    }
}

impl Solar {
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the ambient light level (0 = blinding .. 0xFF = dark).
    /// Wired to core options later; defaults to mid-scale.
    pub fn set_light_level(&mut self, level: u8) {
        self.level = level;
    }

    pub fn light_level(&self) -> u8 {
        self.level
    }

    /// Feed output pin levels after a GPIO write. `clk_fall`/`rst`/`cs`
    /// are the driven line levels; edges are computed by the owner.
    pub fn pins(&mut self, clk_fall: bool, rst: bool, cs: bool) {
        if rst {
            self.counter = 0;
            self.light = self.level;
            return;
        }
        if cs {
            // Selected RTC owns the bus; solar ignores clocks.
            return;
        }
        if clk_fall {
            self.counter = self.counter.wrapping_add(1);
        }
    }

    /// FLG output level for input-pin reads.
    pub fn flag(&self) -> bool {
        self.counter >= self.light
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_clocks_to_flag() {
        let mut s = Solar::new();
        s.set_light_level(3);
        // Reset samples the level before counting.
        s.pins(false, true, false);
        assert!(!s.flag());
        s.pins(true, false, false);
        s.pins(true, false, false);
        assert!(!s.flag());
        s.pins(true, false, false);
        assert!(s.flag());
        // Reset clears.
        s.pins(false, true, false);
        assert!(!s.flag());
    }

    #[test]
    fn cs_high_freezes_counter() {
        let mut s = Solar::new();
        s.set_light_level(1);
        s.pins(true, false, true);
        assert!(!s.flag());
    }
}
