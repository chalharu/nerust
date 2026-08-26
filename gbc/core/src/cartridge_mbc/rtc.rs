use std::time::{SystemTime, UNIX_EPOCH};

pub(super) const RTC_CLOCKS_PER_SECOND: u32 = 4_194_304;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(super) struct RtcRegisters {
    seconds: u8,
    minutes: u8,
    hours: u8,
    days: u16,
    halted: bool,
    day_carry: bool,
}

impl RtcRegisters {
    fn read(self, register: u8) -> u8 {
        match register {
            0x08 => self.seconds,
            0x09 => self.minutes,
            0x0A => self.hours,
            0x0B => self.days as u8,
            0x0C => {
                ((self.days >> 8) as u8 & 0x01)
                    | (u8::from(self.halted) << 6)
                    | (u8::from(self.day_carry) << 7)
            }
            _ => 0xFF,
        }
    }

    fn write(&mut self, register: u8, value: u8) {
        match register {
            0x08 => self.seconds = value & 0x3F,
            0x09 => self.minutes = value & 0x3F,
            0x0A => self.hours = value & 0x1F,
            0x0B => self.days = (self.days & 0x100) | u16::from(value),
            0x0C => {
                self.days = (self.days & 0xFF) | (u16::from(value & 0x01) << 8);
                self.halted = value & 0x40 != 0;
                self.day_carry = value & 0x80 != 0;
            }
            _ => {}
        }
    }

    fn increment_second(&mut self) {
        if self.seconds != 59 {
            self.seconds = self.seconds.wrapping_add(1) & 0x3F;
            return;
        }
        self.seconds = 0;
        if self.minutes != 59 {
            self.minutes = self.minutes.wrapping_add(1) & 0x3F;
            return;
        }
        self.minutes = 0;
        if self.hours != 23 {
            self.hours = self.hours.wrapping_add(1) & 0x1F;
            return;
        }
        self.hours = 0;
        if self.days < 511 {
            self.days += 1;
        } else {
            self.days = 0;
            self.day_carry = true;
        }
    }

    fn advance_seconds(&mut self, mut seconds: u64) {
        if self.halted {
            return;
        }

        while seconds > 0 && (self.seconds > 59 || self.minutes > 59 || self.hours > 23) {
            self.increment_second();
            seconds -= 1;
        }
        if seconds == 0 {
            return;
        }

        let current = u64::from(self.days) * 86_400
            + u64::from(self.hours) * 3_600
            + u64::from(self.minutes) * 60
            + u64::from(self.seconds);
        let total = current + seconds;
        let total_days = total / 86_400;
        if total_days >= 512 {
            self.day_carry = true;
        }
        self.days = (total_days % 512) as u16;
        let day_seconds = total % 86_400;
        self.hours = (day_seconds / 3_600) as u8;
        self.minutes = ((day_seconds % 3_600) / 60) as u8;
        self.seconds = (day_seconds % 60) as u8;
    }

    fn validate(self) -> Result<(), String> {
        if self.seconds > 0x3F || self.minutes > 0x3F || self.hours > 0x1F || self.days > 0x1FF {
            return Err("RTC register value out of range".into());
        }
        Ok(())
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub(super) struct Mbc3Rtc {
    live: RtcRegisters,
    latched: RtcRegisters,
    subsecond_clocks: u32,
    previous_latch_write: u8,
    saved_at_unix_seconds: Option<u64>,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct RtcPersistentState {
    live: RtcRegisters,
    subsecond_clocks: u32,
    saved_at_unix_seconds: u64,
}

impl Mbc3Rtc {
    pub fn new() -> Self {
        Self {
            live: RtcRegisters::default(),
            latched: RtcRegisters::default(),
            subsecond_clocks: 0,
            previous_latch_write: 0xFF,
            saved_at_unix_seconds: None,
        }
    }

    pub fn read_latched(&self, register: u8) -> u8 {
        self.latched.read(register)
    }

    pub fn write_live(&mut self, register: u8, value: u8) {
        self.live.write(register, value);
        if register == 0x08 {
            self.subsecond_clocks = 0;
        }
    }

    pub fn write_latch(&mut self, value: u8) {
        if self.previous_latch_write == 0 && value == 1 {
            self.latched = self.live;
        }
        self.previous_latch_write = value;
    }

    pub fn step_clock(&mut self) {
        if self.live.halted {
            return;
        }
        self.subsecond_clocks += 1;
        if self.subsecond_clocks >= RTC_CLOCKS_PER_SECOND {
            self.subsecond_clocks -= RTC_CLOCKS_PER_SECOND;
            self.live.increment_second();
        }
    }

    pub fn sync(&mut self, now: SystemTime) {
        let Some(saved_at) = self.saved_at_unix_seconds.take() else {
            return;
        };
        let now = unix_seconds(now);
        if now > saved_at && !self.live.halted {
            self.live.advance_seconds(now - saved_at);
        }
    }

    pub fn encode_persistent(&self, now: SystemTime) -> Result<Vec<u8>, String> {
        rmp_serde::to_vec_named(&RtcPersistentState {
            live: self.live,
            subsecond_clocks: self.subsecond_clocks,
            saved_at_unix_seconds: unix_seconds(now),
        })
        .map_err(|error| error.to_string())
    }

    pub fn decode_persistent(data: &[u8]) -> Result<Self, String> {
        let state: RtcPersistentState =
            rmp_serde::from_slice(data).map_err(|error| error.to_string())?;
        state.live.validate()?;
        if state.subsecond_clocks >= RTC_CLOCKS_PER_SECOND {
            return Err("RTC subsecond clock value out of range".into());
        }
        Ok(Self {
            live: state.live,
            latched: state.live,
            subsecond_clocks: state.subsecond_clocks,
            previous_latch_write: 0xFF,
            saved_at_unix_seconds: Some(state.saved_at_unix_seconds),
        })
    }

    pub fn validate(&self) -> Result<(), String> {
        self.live.validate()?;
        self.latched.validate()?;
        if self.subsecond_clocks >= RTC_CLOCKS_PER_SECOND {
            return Err("RTC subsecond clock value out of range".into());
        }
        Ok(())
    }
}

impl Default for Mbc3Rtc {
    fn default() -> Self {
        Self::new()
    }
}

fn unix_seconds(time: SystemTime) -> u64 {
    time.duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    fn latch(rtc: &mut Mbc3Rtc) {
        rtc.write_latch(0);
        rtc.write_latch(1);
    }

    #[test]
    fn second_advances_at_exact_clock_boundary() {
        let mut rtc = Mbc3Rtc::new();
        rtc.subsecond_clocks = RTC_CLOCKS_PER_SECOND - 1;

        rtc.step_clock();
        latch(&mut rtc);

        assert_eq!(rtc.read_latched(0x08), 1);
        assert_eq!(rtc.subsecond_clocks, 0);
    }

    #[test]
    fn registers_carry_through_day_overflow() {
        let mut rtc = Mbc3Rtc::new();
        rtc.write_live(0x08, 59);
        rtc.write_live(0x09, 59);
        rtc.write_live(0x0A, 23);
        rtc.write_live(0x0B, 0xFF);
        rtc.write_live(0x0C, 0x01);
        rtc.subsecond_clocks = RTC_CLOCKS_PER_SECOND - 1;

        rtc.step_clock();
        latch(&mut rtc);

        assert_eq!(rtc.read_latched(0x08), 0);
        assert_eq!(rtc.read_latched(0x09), 0);
        assert_eq!(rtc.read_latched(0x0A), 0);
        assert_eq!(rtc.read_latched(0x0B), 0);
        assert_eq!(rtc.read_latched(0x0C), 0x80);
    }

    #[test]
    fn halt_preserves_subsecond_progress() {
        let mut rtc = Mbc3Rtc::new();
        rtc.subsecond_clocks = RTC_CLOCKS_PER_SECOND - 1;
        rtc.write_live(0x0C, 0x40);
        rtc.step_clock();
        assert_eq!(rtc.subsecond_clocks, RTC_CLOCKS_PER_SECOND - 1);

        rtc.write_live(0x0C, 0);
        rtc.step_clock();
        latch(&mut rtc);

        assert_eq!(rtc.read_latched(0x08), 1);
    }

    #[test]
    fn live_writes_are_hidden_until_next_latch() {
        let mut rtc = Mbc3Rtc::new();
        rtc.write_live(0x09, 0x10);
        latch(&mut rtc);
        rtc.write_live(0x09, 0x20);

        assert_eq!(rtc.read_latched(0x09), 0x10);
        latch(&mut rtc);
        assert_eq!(rtc.read_latched(0x09), 0x20);
    }

    #[test]
    fn only_zero_to_one_sequence_latches() {
        let mut rtc = Mbc3Rtc::new();
        rtc.write_live(0x08, 1);
        rtc.write_latch(1);
        assert_eq!(rtc.read_latched(0x08), 0);

        latch(&mut rtc);
        assert_eq!(rtc.read_latched(0x08), 1);
    }

    #[test]
    fn register_writes_are_masked() {
        let mut rtc = Mbc3Rtc::new();
        for register in 0x08..=0x0C {
            rtc.write_live(register, 0xFF);
        }
        latch(&mut rtc);

        assert_eq!(rtc.read_latched(0x08), 0x3F);
        assert_eq!(rtc.read_latched(0x09), 0x3F);
        assert_eq!(rtc.read_latched(0x0A), 0x1F);
        assert_eq!(rtc.read_latched(0x0B), 0xFF);
        assert_eq!(rtc.read_latched(0x0C), 0xC1);
    }

    #[test]
    fn seconds_write_resets_subsecond_but_minutes_write_does_not() {
        let mut rtc = Mbc3Rtc::new();
        rtc.subsecond_clocks = 123;

        rtc.write_live(0x09, 1);
        assert_eq!(rtc.subsecond_clocks, 123);
        rtc.write_live(0x08, 1);

        assert_eq!(rtc.subsecond_clocks, 0);
    }

    #[test]
    fn out_of_range_minutes_wrap_without_carrying_hours() {
        let mut rtc = Mbc3Rtc::new();
        rtc.write_live(0x09, 0x3F);
        rtc.write_live(0x08, 59);
        rtc.subsecond_clocks = RTC_CLOCKS_PER_SECOND - 1;

        rtc.step_clock();
        latch(&mut rtc);

        assert_eq!(rtc.read_latched(0x09), 0);
        assert_eq!(rtc.read_latched(0x0A), 0);
    }

    #[test]
    fn persistent_sync_applies_elapsed_time_once() {
        let rtc = Mbc3Rtc::new();
        let saved = rtc
            .encode_persistent(UNIX_EPOCH + Duration::from_secs(100))
            .expect("encode");
        let mut restored = Mbc3Rtc::decode_persistent(&saved).expect("decode");

        restored.sync(UNIX_EPOCH + Duration::from_secs(105));
        restored.sync(UNIX_EPOCH + Duration::from_secs(110));
        latch(&mut restored);

        assert_eq!(restored.read_latched(0x08), 5);
    }

    #[test]
    fn backward_clock_does_not_reverse_rtc() {
        let rtc = Mbc3Rtc::new();
        let saved = rtc
            .encode_persistent(UNIX_EPOCH + Duration::from_secs(100))
            .expect("encode");
        let mut restored = Mbc3Rtc::decode_persistent(&saved).expect("decode");

        restored.sync(UNIX_EPOCH + Duration::from_secs(90));
        latch(&mut restored);

        assert_eq!(restored.read_latched(0x08), 0);
    }
}
