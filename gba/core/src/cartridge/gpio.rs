/// GamePak GPIO (General-Purpose I/O) port overlay at 080000C4h-C8h.
///
/// GBATEK `#gbacartioportgpio`: 4-bit bidirectional port used by RTC
/// (S3511), solar sensor (Boktai), tilt/gyro/rumble carts. Data at C4h,
/// direction at C6h, control at C8h (bit 0 = port enable). Control bit 0
/// selects Write-Only vs Read/Write: in write-only mode READS return 00h,
/// but WRITES still latch (only reads are gated). Pre-enable setup bytes
/// therefore survive until the port is enabled.
///
/// Attachment is lazy: the overlay stays dormant (reads return ROM data,
/// writes latch open bus as before) until the first control write with
/// bit 0 set. Games without GPIO hardware never write there (ROM is
/// read-only), so this cannot misfire on normal carts.
///
/// Modeled here: register behavior only. Attached DEVICES (RTC time,
/// solar light level, gyro, rumble) are out of scope: input pins read 0.
/// This already fixes presence detection and control flow for GPIO games;
/// titles needing live sensor data remain unsupported (residual G13).
#[derive(Debug, Default, Clone, Copy)]
pub struct Gpio {
    control: u16,
    direction: u16,
    data: u16,
    attached: bool,
}

impl Gpio {
    pub fn new() -> Self {
        Self::default()
    }

    /// CPU read of the GPIO window. Returns `None` when the access should
    /// fall through to ROM (dormant, misaligned, or disabled-control read
    /// of data/direction which returns 00h per GBATEK — still `Some(0)`).
    pub fn read(&self, addr: u32, width: u8) -> Option<u32> {
        // GBATEK cartridge GPIO: the C4/C6/C8 registers mirror across the
        // WS0/WS1/WS2 ROM regions (08/0A/0C). ROM-bus accesses are 16/32-bit
        // (STRB opcodes ignored); a 32-bit read composes two halves.
        match width {
            2 => self.read_half(addr).map(u32::from),
            4 => {
                let lo = self.read_half(addr)?;
                let hi = self.read_half(addr.wrapping_add(2)).unwrap_or(0);
                Some(lo as u32 | ((hi as u32) << 16))
            }
            _ => None,
        }
    }

    fn read_half(&self, addr: u32) -> Option<u16> {
        if !matches!(
            addr,
            0x080000C4
                | 0x080000C6
                | 0x080000C8
                | 0x0A0000C4
                | 0x0A0000C6
                | 0x0A0000C8
                | 0x0C0000C4
                | 0x0C0000C6
                | 0x0C0000C8
        ) {
            return None;
        }
        if !self.attached {
            return None;
        }
        Some(match addr & 0xFF {
            0xC8 => self.control,
            _ if self.control & 1 == 0 => 0,
            0xC4 => self.data & self.direction,
            _ => self.direction & 0xF,
        })
    }

    /// CPU write to the GPIO window. Returns true when consumed (control
    /// writes always; data/direction writes latch always but only consume
    /// once enabled, keeping non-GPIO carts on the ROM path).
    pub fn write(&mut self, addr: u32, width: u8, value: u32) -> bool {
        match width {
            2 => self.write_half(addr, (value & 0xFFFF) as u16),
            // A 32-bit write splits into two halfword writes; consumed if
            // either half hits a register.
            4 => {
                let lo = self.write_half(addr, (value & 0xFFFF) as u16);
                let hi = self.write_half(addr.wrapping_add(2), (value >> 16) as u16);
                lo || hi
            }
            _ => false,
        }
    }

    fn write_half(&mut self, addr: u32, v: u16) -> bool {
        // Only exact C4/C6/C8 halfwords (per mirror) are registers; other
        // halves of a split 32-bit write fall through to ROM.
        if !matches!(
            addr,
            0x080000C4
                | 0x080000C6
                | 0x080000C8
                | 0x0A0000C4
                | 0x0A0000C6
                | 0x0A0000C8
                | 0x0C0000C4
                | 0x0C0000C6
                | 0x0C0000C8
        ) {
            return false;
        }
        if addr & 0xFF == 0xC8 {
            self.control = v & 1;
            if self.control & 1 != 0 {
                self.attached = true;
            }
            return true;
        }
        // GBATEK: control bit 0 gates READS (00h in write-only mode), not
        // writes — latch data/direction unconditionally so pre-enable
        // setup survives. Attachment still waits for the enable write so
        // non-GPIO carts never misfire (their reads keep hitting ROM).
        if addr & 0xFF == 0xC4 {
            self.data = v & 0xF;
        } else {
            self.direction = v & 0xF;
        }
        if self.control & 1 != 0 {
            self.attached = true;
            return true;
        }
        false
    }

    /// Whether any GPIO register has been enabled (for tests).
    pub fn is_attached(&self) -> bool {
        self.attached
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dormant_until_control_enable() {
        let mut gpio = Gpio::new();
        // Reads fall through to ROM while dormant.
        assert_eq!(gpio.read(0x080000C4, 2), None);
        // Pre-enable data writes latch (GBATEK: write-only gates reads,
        // not writes) but do not attach yet.
        assert!(!gpio.write(0x080000C4, 2, 0xF));
        assert!(!gpio.is_attached());
        // Enabling attaches, and the pre-enable byte survived (made
        // visible by opening the direction pins).
        assert!(gpio.write(0x080000C8, 2, 1));
        assert!(gpio.is_attached());
        assert_eq!(gpio.read(0x080000C8, 2), Some(1));
        assert!(gpio.write(0x080000C6, 2, 0xF));
        assert_eq!(gpio.read(0x080000C4, 2), Some(0xF));
    }

    #[test]
    fn direction_masks_data_reads() {
        let mut gpio = Gpio::new();
        gpio.write(0x080000C8, 2, 1);
        gpio.write(0x080000C6, 2, 0b1010);
        gpio.write(0x080000C4, 2, 0b1111);
        // Input pins (no device) read 0.
        assert_eq!(gpio.read(0x080000C4, 2), Some(0b1010));
        assert_eq!(gpio.read(0x080000C6, 2), Some(0b1010));
    }

    #[test]
    fn disabled_control_reads_zero() {
        let mut gpio = Gpio::new();
        gpio.write(0x080000C8, 2, 1);
        gpio.write(0x080000C4, 2, 0xF);
        gpio.write(0x080000C8, 2, 0);
        assert_eq!(gpio.read(0x080000C4, 2), Some(0));
        assert_eq!(gpio.read(0x080000C8, 2), Some(0));
    }

    #[test]
    fn wrong_width_or_address_falls_through() {
        let gpio = Gpio::new();
        assert_eq!(gpio.read(0x080000C4, 1), None);
        assert_eq!(gpio.read(0x080000C0, 2), None);
        // Dormant 32-bit reads fall through as well.
        assert_eq!(gpio.read(0x080000C4, 4), None);
    }

    #[test]
    fn thirty_two_bit_access_composes_halves() {
        // GBATEK: ROM-bus GPIO works with 16/32-bit accesses (STRB ignored).
        let mut gpio = Gpio::new();
        assert!(gpio.write(0x080000C8, 4, 0x0000_0001));
        assert!(gpio.is_attached());
        assert!(gpio.write(0x080000C4, 4, 0xFFFF_FFFF));
        // Low half (C4) = data&direction, high half (C6) = direction.
        assert_eq!(gpio.read(0x080000C6, 2), Some(0xF));
        assert_eq!(gpio.read(0x080000C4, 4), Some(0x000F_000F));
        // Control composes with the zero-filled byte past C8.
        assert_eq!(gpio.read(0x080000C8, 4), Some(0x0000_0001));
    }
}
