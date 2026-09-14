/// GamePak RTC (Seiko S3511 3-wire serial) after NBA `HW/GamePak/GPIO/RTC`,
/// cross-checked against GBAHawk `Mappers.h`, GBATEK "Game Pak RTC".
///
/// Pins: 0 = SCK, 1 = SIO, 2 = CS. CS low aborts; the command byte is
/// clocked LSB-first on SCK rises (MSB-first senders are auto-detected
/// by the `0110b` magic, NBA-style). Data bytes shift out on SCK falls.
#[derive(Debug, Default, Clone, Copy)]
pub struct Rtc {
    phase: Phase,
    cmd: u8,
    bits: u8,
    param: [u8; 8],
    param_len: u8,
    out: [u8; 8],
    out_len: u8,
    out_bit: u8,
    is_read: bool,
    /// Stored control register (bit6 = 12/24h, bit7 = poweroff).
    pub control: u8,
    /// Host-time override from datetime/time writes (binary fields).
    datetime_set: Option<[u8; 7]>,
    /// SIO level currently driven for input-pin reads.
    pub sio_out: bool,
}

/// Argument byte counts per command (NBA/GBAHawk agree).
const ARG_COUNT: [u8; 8] = [0, 0, 7, 0, 1, 0, 3, 0];

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
enum Phase {
    #[default]
    Idle,
    Command,
    Receiving,
    Sending,
}

fn reverse_bits(mut v: u8) -> u8 {
    let mut out = 0u8;
    for _ in 0..8 {
        out = (out << 1) | (v & 1);
        v >>= 1;
    }
    out
}

impl Rtc {
    /// Feed current pin levels after a GPIO write. Returns nothing; the
    /// driven SIO level is visible in `sio_out`.
    pub fn pins(&mut self, sck: bool, sio: bool, cs: bool, prev_sck: bool, prev_cs: bool) {
        if !cs {
            // CS low aborts any transaction.
            if prev_cs {
                self.phase = Phase::Idle;
                self.sio_out = false;
            }
            return;
        }
        if !prev_cs {
            // CS rise starts a command byte.
            self.phase = Phase::Command;
            self.cmd = 0;
            self.bits = 0;
            self.sio_out = false;
        }
        if self.phase == Phase::Command && sck && !prev_sck {
            self.cmd |= (u8::from(sio)) << self.bits;
            self.bits += 1;
            if self.bits == 8 {
                self.decode_command();
            }
        } else if self.phase == Phase::Receiving && sck && !prev_sck {
            let i = (self.bits / 8) as usize;
            self.param[i] |= (u8::from(sio)) << (self.bits % 8);
            self.bits += 1;
            if self.bits / 8 >= self.param_len {
                self.apply_write();
                self.phase = Phase::Idle;
            }
        } else if self.phase == Phase::Sending && prev_sck && !sck {
            // Data shifts out on the falling edge.
            let i = (self.out_bit / 8) as usize;
            self.sio_out = self.out[i] >> (self.out_bit % 8) & 1 != 0;
            self.out_bit += 1;
            if self.out_bit / 8 >= self.out_len {
                self.phase = Phase::Idle;
            }
        }
    }

    fn decode_command(&mut self) {
        // Accept LSB-first (`xxxx0110`) or MSB-first (bit-reversed) senders.
        let cmd = if self.cmd & 0x0F == 0x06 {
            self.cmd
        } else if reverse_bits(self.cmd) & 0x0F == 0x06 {
            reverse_bits(self.cmd)
        } else {
            self.phase = Phase::Idle;
            return;
        };
        let num = ((cmd >> 4) & 7) as usize;
        self.is_read = cmd & 0x80 != 0;
        let args = ARG_COUNT[num];
        match (num, self.is_read) {
            (0, _) => {
                // Reset: control clears (GBATEK post-reset 00h).
                self.control = 0;
                self.phase = Phase::Idle;
            }
            (3, _) => {
                // ForceIRQ: strobed by access; no state to keep.
                self.phase = Phase::Idle;
            }
            (_, true) => {
                self.prepare_read(num);
                self.phase = Phase::Sending;
                self.out_bit = 0;
                self.sio_out = false;
            }
            (_, false) if args == 0 => {
                self.phase = Phase::Idle;
            }
            _ => {
                self.param = [0; 8];
                self.param_len = args;
                self.bits = 0;
                self.phase = Phase::Receiving;
            }
        }
    }

    fn prepare_read(&mut self, num: usize) {
        self.out = [0; 8];
        match num {
            2 => {
                let dt = self.datetime();
                self.out[..7].copy_from_slice(&dt);
                self.out_len = 7;
            }
            6 => {
                let dt = self.datetime();
                self.out[..3].copy_from_slice(&dt[4..7]);
                self.out_len = 3;
            }
            4 => {
                self.out[0] = self.control;
                self.out_len = 1;
            }
            _ => {
                self.out_len = 0;
            }
        }
    }

    fn apply_write(&mut self) {
        let from_bcd = |b: u8| (b >> 4) * 10 + (b & 0xF);
        // Command number is recoverable from the decoded length.
        match self.param_len {
            7 => {
                // Datetime write: BCD yy,mm,dd,dow,hh,mm,ss.
                let mut dt = [0u8; 7];
                for (i, b) in dt.iter_mut().enumerate() {
                    *b = from_bcd(self.param[i]);
                }
                // GBA hour bit 7 is AM/PM in 12h mode; store 24h binary.
                if self.control & (1 << 6) == 0 && dt[4] & 0x80 != 0 {
                    dt[4] = (dt[4] & 0x7F) % 12 + 12;
                } else {
                    dt[4] &= 0x7F;
                }
                self.datetime_set = Some(dt);
            }
            3 => {
                // Time write: BCD hh,mm,ss replaces the time fields.
                let mut dt = self.datetime_binary();
                for (i, b) in dt[4..7].iter_mut().enumerate() {
                    *b = from_bcd(self.param[i]);
                }
                self.datetime_set = Some(dt);
            }
            1 => {
                self.control = self.param[0];
            }
            _ => {}
        }
    }

    /// Current datetime as BCD (GBATEK order yy,mm,dd,dow,hh,mm,ss).
    fn datetime(&self) -> [u8; 7] {
        let bin = self.datetime_binary();
        let to_bcd = |v: u8| ((v / 10) << 4) | (v % 10);
        let mut out = [0u8; 7];
        for (i, b) in out.iter_mut().enumerate() {
            *b = to_bcd(bin[i]);
        }
        // GBA 12h mode reports PM in hour bit 7 (GBATEK).
        if self.control & (1 << 6) == 0 {
            let h = bin[4] % 24;
            out[4] = to_bcd(h % 12) | if h >= 12 { 0x80 } else { 0 };
            if h.is_multiple_of(12) {
                out[4] = to_bcd(12) | if h >= 12 { 0x80 } else { 0 };
            }
        }
        out
    }

    fn datetime_binary(&self) -> [u8; 7] {
        if let Some(dt) = self.datetime_set {
            return dt;
        }
        host_datetime_utc()
    }
}

/// Host clock as binary [yy,mm,dd,dow,hh,mm,ss] in UTC (civil-from-days;
/// no timezone database is available in core; UTC is documented).
fn host_datetime_utc() -> [u8; 7] {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let days = (secs / 86_400) as i64;
    let tod = secs % 86_400;
    // Howard Hinnant's civil_from_days.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u8;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u8;
    let y = (if m <= 2 { y + 1 } else { y }) as u32;
    // 1970-01-01 was a Thursday.
    let dow = ((days + 4).rem_euclid(7)) as u8;
    [
        (y % 100) as u8,
        m,
        d,
        dow,
        (tod / 3_600) as u8,
        ((tod % 3_600) / 60) as u8,
        (tod % 60) as u8,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn clock_bits(rtc: &mut Rtc, sio: bool, cs: bool, prev_sck: bool, prev_cs: bool) {
        rtc.pins(true, sio, cs, prev_sck, prev_cs);
        rtc.pins(false, sio, cs, true, cs);
    }

    fn send_byte(rtc: &mut Rtc, byte: u8) {
        for i in 0..8 {
            clock_bits(rtc, byte >> i & 1 != 0, true, false, true);
        }
    }

    #[test]
    fn reset_command_clears_control() {
        let mut rtc = Rtc {
            control: 0x40,
            ..Default::default()
        };
        rtc.pins(false, false, true, false, false);
        // LSB-first 0x06|0x00: magic 0110, cmd 0, write.
        send_byte(&mut rtc, 0x06);
        assert_eq!(rtc.control, 0);
    }

    #[test]
    fn control_write_and_read_round_trip() {
        let mut rtc = Rtc::default();
        rtc.pins(false, false, true, false, false);
        // Write control (cmd 4): 0x46.
        send_byte(&mut rtc, 0x46);
        // Param byte 0x40.
        send_byte(&mut rtc, 0x40);
        assert_eq!(rtc.control, 0x40);
        // CS drop + rise restarts; read control: 0x46|0x80 = 0xC6.
        rtc.pins(false, false, false, false, true);
        rtc.pins(false, false, true, false, false);
        send_byte(&mut rtc, 0xC6);
        // Data shifts out on falls; the master samples after each rise.
        let mut got = 0u8;
        for i in 0..8 {
            rtc.pins(true, false, true, false, true);
            got |= u8::from(rtc.sio_out) << i;
            rtc.pins(false, false, true, true, true);
        }
        assert_eq!(got, 0x40);
    }

    #[test]
    fn datetime_write_feeds_read() {
        let mut rtc = Rtc::default();
        rtc.pins(false, false, true, false, false);
        // Write datetime (cmd 2): 0x26, then 7 BCD bytes.
        send_byte(&mut rtc, 0x26);
        for b in [0x24u8, 0x01, 0x02, 0x03, 0x15, 0x30, 0x45] {
            send_byte(&mut rtc, b);
        }
        // Read datetime back: 0x26|0x80 = 0xA6.
        rtc.pins(false, false, false, false, true);
        rtc.pins(false, false, true, false, false);
        send_byte(&mut rtc, 0xA6);
        let mut got = [0u8; 7];
        for g in got.iter_mut() {
            let mut v = 0u8;
            for i in 0..8 {
                rtc.pins(true, false, true, false, true);
                v |= u8::from(rtc.sio_out) << i;
                rtc.pins(false, false, true, true, true);
            }
            *g = v;
        }
        assert_eq!(got[0], 0x24);
        assert_eq!(got[5], 0x30);
    }
}
