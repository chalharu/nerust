/// GamePak GPIO (General-Purpose I/O) port overlay at 080000C4h-C8h.
///
/// GBATEK `#gbacartioportgpio`: 4-bit bidirectional port used by RTC
/// (S3511), solar sensor (Boktai), tilt/gyro/rumble carts. Data at C4h,
/// direction at C6h, control at C8h (bit 0 = port enable). Data and
/// direction are only accessible while control bit 0 is set; otherwise
/// reads return 00h and writes are ignored.
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
        // WS0/WS1/WS2 ROM regions (08/0A/0C).
        if width != 2
            || !matches!(
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
            )
        {
            return None;
        }
        if !self.attached {
            return None;
        }
        Some(match addr & 0xFF {
            0xC8 => u32::from(self.control),
            _ if self.control & 1 == 0 => 0,
            0xC4 => u32::from(self.data & self.direction),
            _ => u32::from(self.direction & 0xF),
        })
    }

    /// CPU write to the GPIO window. Returns true when consumed (control
    /// writes always attach on bit 0; data/direction writes need enable).
    pub fn write(&mut self, addr: u32, width: u8, value: u32) -> bool {
        if width != 2
            || !matches!(
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
            )
        {
            return false;
        }
        let v = (value & 0xFFFF) as u16;
        if addr & 0xFF == 0xC8 {
            self.control = v & 1;
            if self.control & 1 != 0 {
                self.attached = true;
            }
            return true;
        }
        if self.control & 1 == 0 {
            return false;
        }
        self.attached = true;
        if addr & 0xFF == 0xC4 {
            self.data = v & 0xF;
        } else {
            self.direction = v & 0xF;
        }
        true
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
        // Data writes without enable are ignored (open bus path).
        assert!(!gpio.write(0x080000C4, 2, 0xF));
        assert!(!gpio.is_attached());
        // Enabling attaches.
        assert!(gpio.write(0x080000C8, 2, 1));
        assert!(gpio.is_attached());
        assert_eq!(gpio.read(0x080000C8, 2), Some(1));
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
        assert_eq!(gpio.read(0x080000C4, 4), None);
        assert_eq!(gpio.read(0x080000C0, 2), None);
    }
}
