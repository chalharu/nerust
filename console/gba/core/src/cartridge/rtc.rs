/// GamePak RTC (Seiko S3511 3-wire serial; GBATEK "Game Pak RTC" plus the
/// S-35180 command detail; behavior cross-checked against HW test ROMs).
///
/// Pins: 0 = SCK, 1 = SIO, 2 = CS. CS low aborts; the command byte is
/// clocked LSB-first on SCK rises (MSB-first senders are auto-detected
/// by the `0110b` magic). Data bytes shift out on SCK falls.
#[derive(Debug, Default, Clone, Copy, serde::Serialize, serde::Deserialize)]
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
    /// Wall-clock second when `datetime_set` was written. Reads report
    /// `datetime_set + (now - set_at)`, so the clock keeps ticking from
    /// the game-set value like the hardware (and keeps running across
    /// State Save/Load). `None` on pre-fix states: those keep the legacy
    /// frozen behavior. Missing on decode (serde `Option` default).
    set_at_unix_secs: Option<u64>,
    /// SIO level currently driven for input-pin reads.
    pub sio_out: bool,
}

/// Argument byte counts per command (0,0,7,0,1,0,3,0 per GBATEK).
const ARG_COUNT: [u8; 8] = [0, 0, 7, 0, 1, 0, 3, 0];

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
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
    /// Phase 10 import validation (bounds follow the protocol engine).
    pub(super) fn validate(&self) -> Result<(), String> {
        // `bits` counts incoming edges: up to 8 in a command, up to 56
        // while receiving (ARG_COUNT max 7, and the `param[bits / 8]`
        // index needs `bits / 8 < 8`), and a CS-low abort can strand a
        // mid-receive count in Idle. `out_bit` shifts the response out:
        // below `out_len * 8` while sending.
        if self.param_len > 8 {
            return Err(format!(
                "rtc: param length out of range: {}",
                self.param_len
            ));
        }
        if self.out_len > 8 {
            return Err(format!("rtc: output length out of range: {}", self.out_len));
        }
        let bits_max = match self.phase {
            Phase::Receiving | Phase::Idle => 56,
            Phase::Command | Phase::Sending => 8,
        };
        if usize::from(self.bits) > bits_max {
            return Err(format!("rtc: bit counter out of range: {}", self.bits));
        }
        if self.out_bit as usize > 64 {
            return Err(format!("rtc: output bit out of range: {}", self.out_bit));
        }
        if self.phase == Phase::Sending && (self.out_bit as usize) >= usize::from(self.out_len) * 8
        {
            return Err("rtc: output bit past response end while sending".to_string());
        }
        Ok(())
    }

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
                self.set_at_unix_secs = Some(unix_now_secs());
            }
            3 => {
                // Time write: BCD hh,mm,ss replaces the time fields.
                let mut dt = self.datetime_binary();
                for (i, b) in dt[4..7].iter_mut().enumerate() {
                    *b = from_bcd(self.param[i]);
                }
                self.datetime_set = Some(dt);
                self.set_at_unix_secs = Some(unix_now_secs());
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
        match (self.datetime_set, self.set_at_unix_secs) {
            (Some(set), Some(set_at)) => advance_binary_datetime(set, set_at, unix_now_secs()),
            // Pre-fix states carry no stamp: keep the legacy frozen value.
            (Some(set), None) => set,
            (None, _) => host_datetime_utc(),
        }
    }
}

/// Game-set datetime advanced by wall-clock elapsed seconds. `dt` is
/// binary [yy,mm,dd,dow,hh,mm,ss] (24h); the two-digit year is read in
/// the 2000s. A clock running backward (or no elapsed time) reports the
/// set value unchanged, mirroring the GBC backward-clock guard.
fn advance_binary_datetime(dt: [u8; 7], set_at_unix_secs: u64, now_unix_secs: u64) -> [u8; 7] {
    let elapsed = now_unix_secs.saturating_sub(set_at_unix_secs);
    if elapsed == 0 {
        return dt;
    }
    let base_days = days_from_civil(2000 + i64::from(dt[0]), dt[1], dt[2]);
    let base_tod = i64::from(dt[4]) * 3_600 + i64::from(dt[5]) * 60 + i64::from(dt[6]);
    let total = base_days * 86_400 + base_tod + elapsed as i64;
    let days = total.div_euclid(86_400);
    let tod = total.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    [
        (year % 100) as u8,
        month,
        day,
        day_of_week(days),
        (tod / 3_600) as u8,
        ((tod % 3_600) / 60) as u8,
        (tod % 60) as u8,
    ]
}

/// Howard Hinnant's days_from_civil (days since 1970-01-01).
fn days_from_civil(year: i64, month: u8, day: u8) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let yoe = year - era * 400;
    let mp = (i64::from(month) + 9) % 12;
    let doy = (153 * mp + 2) / 5 + i64::from(day) - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// Howard Hinnant's civil_from_days (full year, 1-based month/day).
fn civil_from_days(days: i64) -> (i64, u8, u8) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u8;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u8;
    let y = if m <= 2 { y + 1 } else { y };
    (y, m, d)
}

/// 1970-01-01 was a Thursday; GBA dow is 0=Sunday..6=Saturday.
fn day_of_week(days_since_epoch: i64) -> u8 {
    ((days_since_epoch + 4).rem_euclid(7)) as u8
}

fn unix_now_secs() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Host clock as binary [yy,mm,dd,dow,hh,mm,ss] in UTC (civil-from-days;
/// no timezone database is available in core; UTC is documented).
fn host_datetime_utc() -> [u8; 7] {
    let secs = unix_now_secs();
    let days = (secs / 86_400) as i64;
    let tod = secs % 86_400;
    let (year, month, day) = civil_from_days(days);
    [
        (year % 100) as u8,
        month,
        day,
        day_of_week(days),
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

    #[test]
    fn validate_accepts_mid_transaction_states() {
        let mut rtc = Rtc::default();
        rtc.validate().unwrap();
        rtc.pins(false, false, true, false, false);
        // Datetime write (cmd 2, 7 param bytes): stop after 3 bytes,
        // leaving bits = 24 mid-Receiving.
        send_byte(&mut rtc, 0x26);
        for b in [0x24u8, 0x01, 0x02] {
            send_byte(&mut rtc, b);
        }
        assert_eq!(rtc.phase, Phase::Receiving);
        assert_eq!(rtc.bits, 24);
        rtc.validate().unwrap();
        // A CS-low abort strands the count in Idle: still valid.
        rtc.pins(false, false, false, false, true);
        assert_eq!(rtc.phase, Phase::Idle);
        rtc.validate().unwrap();
    }

    #[test]
    fn validate_rejects_index_unsafe_states() {
        let bad = Rtc {
            phase: Phase::Receiving,
            bits: 57,
            ..Default::default()
        };
        assert!(bad.validate().is_err());
        // param[bits/8] would panic at bits = 64.
        let bad = Rtc {
            phase: Phase::Receiving,
            bits: 64,
            ..Default::default()
        };
        assert!(bad.validate().is_err());
        let bad = Rtc {
            phase: Phase::Sending,
            out_len: 1,
            out_bit: 8,
            ..Default::default()
        };
        assert!(bad.validate().is_err());
        // out[out_bit/8] would panic at out_bit = 64.
        let bad = Rtc {
            phase: Phase::Sending,
            out_len: 8,
            out_bit: 64,
            ..Default::default()
        };
        assert!(bad.validate().is_err());
        let bad = Rtc {
            param_len: 9,
            ..Default::default()
        };
        assert!(bad.validate().is_err());
    }

    #[test]
    fn advance_binary_datetime_keeps_value_without_elapsed_time() {
        let dt = [24u8, 1, 2, 2, 10, 30, 45];
        assert_eq!(advance_binary_datetime(dt, 1_000_000, 1_000_000), dt);
    }

    #[test]
    fn advance_binary_datetime_rolls_time_and_date() {
        // 2024-01-02 (Tue) 23:59:30 + 90s -> 2024-01-03 (Wed) 00:01:00.
        assert_eq!(
            advance_binary_datetime([24, 1, 2, 2, 23, 59, 30], 1_000_000, 1_000_090),
            [24, 1, 3, 3, 0, 1, 0]
        );
        // 2024-12-31 (Tue) 23:59:30 + 90s -> 2025-01-01 (Wed) 00:01:00.
        assert_eq!(
            advance_binary_datetime([24, 12, 31, 2, 23, 59, 30], 1_000_000, 1_000_090),
            [25, 1, 1, 3, 0, 1, 0]
        );
    }

    #[test]
    fn advance_binary_datetime_handles_leap_years() {
        // 2024-02-28 (Wed) 12:00:00 + 1 day -> Feb 29 (Thu).
        assert_eq!(
            advance_binary_datetime([24, 2, 28, 3, 12, 0, 0], 1_000_000, 1_086_400),
            [24, 2, 29, 4, 12, 0, 0]
        );
        // + 2 days -> Mar 1 (Fri).
        assert_eq!(
            advance_binary_datetime([24, 2, 28, 3, 12, 0, 0], 1_000_000, 1_172_800),
            [24, 3, 1, 5, 12, 0, 0]
        );
        // 2023 is not a leap year: Feb 28 (Tue) + 1 day -> Mar 1 (Wed).
        assert_eq!(
            advance_binary_datetime([23, 2, 28, 2, 12, 0, 0], 1_000_000, 1_086_400),
            [23, 3, 1, 3, 12, 0, 0]
        );
    }

    #[test]
    fn advance_binary_datetime_ignores_backward_clock() {
        let dt = [24u8, 5, 6, 1, 8, 0, 0];
        assert_eq!(advance_binary_datetime(dt, 1_000_100, 1_000_000), dt);
    }

    #[test]
    fn civil_round_trip_is_identity() {
        for days in (-730_000..730_000).step_by(997) {
            let (y, m, d) = civil_from_days(days);
            assert_eq!(days_from_civil(y, m, d), days, "days={days}");
        }
        // Leap-day boundaries both sides of the epoch.
        for days in [59, 60, 61, -30, -31, 365, 366, 15_218, 15_219] {
            let (y, m, d) = civil_from_days(days);
            assert_eq!(days_from_civil(y, m, d), days, "days={days}");
        }
    }

    #[test]
    fn stamped_override_ticks_with_wall_clock() {
        let now = unix_now_secs();
        let rtc = Rtc {
            datetime_set: Some([24, 1, 2, 2, 10, 0, 0]),
            set_at_unix_secs: Some(now - 90),
            ..Default::default()
        };
        assert_eq!(rtc.datetime_binary(), [24, 1, 2, 2, 10, 1, 30]);
    }

    #[test]
    fn unstamped_override_keeps_legacy_frozen_value() {
        let rtc = Rtc {
            datetime_set: Some([24, 1, 2, 2, 10, 0, 0]),
            set_at_unix_secs: None,
            ..Default::default()
        };
        assert_eq!(rtc.datetime_binary(), [24, 1, 2, 2, 10, 0, 0]);
    }

    #[test]
    fn datetime_write_records_stamp() {
        let mut rtc = Rtc::default();
        rtc.pins(false, false, true, false, false);
        send_byte(&mut rtc, 0x26);
        for b in [0x24u8, 0x01, 0x02, 0x03, 0x15, 0x30, 0x45] {
            send_byte(&mut rtc, b);
        }
        assert_eq!(
            rtc.datetime_set,
            Some([24u8, 1, 2, 3, 15, 30, 45]),
            "datetime write stores 24h binary"
        );
        assert!(
            rtc.set_at_unix_secs.is_some_and(|set_at| {
                let now = unix_now_secs();
                set_at <= now && now.saturating_sub(set_at) < 60
            }),
            "datetime write stamps wall-clock time"
        );
    }

    #[test]
    fn stamp_survives_serde_round_trip() {
        let rtc = Rtc {
            control: 0x40,
            datetime_set: Some([24, 1, 2, 2, 10, 0, 0]),
            set_at_unix_secs: Some(1_700_000_000),
            ..Default::default()
        };
        let bytes = rmp_serde::to_vec_named(&rtc).expect("encode");
        let restored: Rtc = rmp_serde::from_slice(&bytes).expect("decode");
        assert_eq!(restored.control, rtc.control);
        assert_eq!(restored.datetime_set, rtc.datetime_set);
        assert_eq!(restored.set_at_unix_secs, rtc.set_at_unix_secs);
    }

    #[test]
    fn pre_fix_states_decode_without_stamp() {
        // States written before the stamp field existed must still load,
        // falling back to the legacy frozen override.
        #[derive(serde::Serialize)]
        struct LegacyRtc {
            phase: Phase,
            cmd: u8,
            bits: u8,
            param: [u8; 8],
            param_len: u8,
            out: [u8; 8],
            out_len: u8,
            out_bit: u8,
            is_read: bool,
            control: u8,
            datetime_set: Option<[u8; 7]>,
            sio_out: bool,
        }
        let legacy = LegacyRtc {
            phase: Phase::Idle,
            cmd: 0,
            bits: 0,
            param: [0; 8],
            param_len: 0,
            out: [0; 8],
            out_len: 0,
            out_bit: 0,
            is_read: false,
            control: 0,
            datetime_set: Some([24, 1, 2, 2, 10, 0, 0]),
            sio_out: false,
        };
        let bytes = rmp_serde::to_vec_named(&legacy).expect("encode");
        let restored: Rtc = rmp_serde::from_slice(&bytes).expect("decode");
        assert_eq!(restored.set_at_unix_secs, None);
        assert_eq!(restored.datetime_binary(), [24, 1, 2, 2, 10, 0, 0]);
    }
}
