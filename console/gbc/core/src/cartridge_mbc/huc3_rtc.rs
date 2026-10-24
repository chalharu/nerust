use std::time::{SystemTime, UNIX_EPOCH};

pub(super) const RTC_MEMORY_SIZE: usize = 0x100;
pub(super) const CLOCKS_PER_MINUTE: u32 = 4_194_304 * 60;

const TRANSFER_START: usize = 0x00;
const LIVE_MINUTES_START: usize = 0x10;
const LIVE_DAYS_START: usize = 0x13;

#[derive(Debug, Clone)]
pub(super) struct HuC3Rtc {
    memory: Vec<u8>,
    subminute_clocks: u32,
    saved_at_unix_seconds: Option<u64>,
}

impl HuC3Rtc {
    pub fn new() -> Self {
        Self {
            memory: vec![0; RTC_MEMORY_SIZE],
            subminute_clocks: 0,
            saved_at_unix_seconds: None,
        }
    }

    pub fn from_state(
        memory: Vec<u8>,
        subminute_clocks: u32,
        saved_at_unix_seconds: Option<u64>,
    ) -> Result<Self, String> {
        let rtc = Self {
            memory,
            subminute_clocks,
            saved_at_unix_seconds,
        };
        rtc.validate()?;
        Ok(rtc)
    }

    pub fn memory(&self) -> &[u8] {
        &self.memory
    }

    pub fn subminute_clocks(&self) -> u32 {
        self.subminute_clocks
    }

    pub fn read(&self, address: u8) -> u8 {
        self.memory[usize::from(address)]
    }

    pub fn write(&mut self, address: u8, value: u8) {
        self.memory[usize::from(address)] = value & 0x0F;
    }

    pub fn copy_current_to_transfer(&mut self) {
        self.memory
            .copy_within(LIVE_MINUTES_START..=0x16, TRANSFER_START);
    }

    pub fn copy_transfer_to_current(&mut self) -> bool {
        let minutes = decode_nibbles(&self.memory[TRANSFER_START..TRANSFER_START + 3]);
        if minutes >= 1_440 {
            return false;
        }
        self.memory
            .copy_within(TRANSFER_START..=0x06, LIVE_MINUTES_START);
        true
    }

    pub fn step_clock(&mut self) {
        self.subminute_clocks += 1;
        if self.subminute_clocks == CLOCKS_PER_MINUTE {
            self.subminute_clocks = 0;
            self.advance_minutes(1);
        }
    }

    pub fn sync(&mut self, now: SystemTime) {
        let Some(saved_at) = self.saved_at_unix_seconds.take() else {
            return;
        };
        self.sync_from_unix_seconds(saved_at, now);
    }

    pub fn sync_from(&mut self, saved_at: SystemTime, now: SystemTime) {
        self.sync_from_unix_seconds(unix_seconds(saved_at), now);
    }

    fn sync_from_unix_seconds(&mut self, saved_at: u64, now: SystemTime) {
        let now = unix_seconds(now);
        if now <= saved_at {
            return;
        }

        let elapsed = now - saved_at;
        let whole_minutes = elapsed / 60;
        let remaining_clocks = (elapsed % 60) * 4_194_304;
        let clock_total = u64::from(self.subminute_clocks) + remaining_clocks;
        self.subminute_clocks = (clock_total % u64::from(CLOCKS_PER_MINUTE)) as u32;
        self.advance_minutes(whole_minutes + clock_total / u64::from(CLOCKS_PER_MINUTE));
    }

    fn advance_minutes(&mut self, elapsed: u64) {
        let current_minutes = u64::from(self.live_minutes());
        let total_minutes = current_minutes + elapsed;
        self.set_live_minutes((total_minutes % 1_440) as u16);

        let elapsed_days = total_minutes / 1_440;
        self.set_live_days(self.live_days().wrapping_add(elapsed_days as u16));
    }

    fn live_minutes(&self) -> u16 {
        decode_nibbles(&self.memory[LIVE_MINUTES_START..LIVE_MINUTES_START + 3]) as u16
    }

    fn set_live_minutes(&mut self, value: u16) {
        encode_nibbles(
            value,
            &mut self.memory[LIVE_MINUTES_START..LIVE_MINUTES_START + 3],
        );
    }

    fn live_days(&self) -> u16 {
        decode_nibbles(&self.memory[LIVE_DAYS_START..LIVE_DAYS_START + 4]) as u16
    }

    fn set_live_days(&mut self, value: u16) {
        encode_nibbles(
            value,
            &mut self.memory[LIVE_DAYS_START..LIVE_DAYS_START + 4],
        );
    }

    fn validate(&self) -> Result<(), String> {
        if self.memory.len() != RTC_MEMORY_SIZE {
            return Err(format!(
                "HuC3 RTC memory length mismatch: expected {RTC_MEMORY_SIZE}, got {}",
                self.memory.len()
            ));
        }
        if self.memory.iter().any(|value| value & 0xF0 != 0) {
            return Err("HuC3 RTC memory contains a non-nibble value".into());
        }
        if self.subminute_clocks >= CLOCKS_PER_MINUTE {
            return Err("HuC3 RTC subminute clock value out of range".into());
        }
        Ok(())
    }
}

fn decode_nibbles(values: &[u8]) -> u64 {
    values.iter().enumerate().fold(0, |result, (shift, value)| {
        result | (u64::from(*value) << (shift * 4))
    })
}

fn encode_nibbles(mut value: u16, output: &mut [u8]) {
    for nibble in output {
        *nibble = value as u8 & 0x0F;
        value >>= 4;
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

    #[test]
    fn advances_at_the_exact_minute_boundary() {
        let mut rtc = HuC3Rtc::new();
        rtc.subminute_clocks = CLOCKS_PER_MINUTE - 1;

        rtc.step_clock();

        assert_eq!(rtc.live_minutes(), 1);
        assert_eq!(rtc.subminute_clocks, 0);
    }

    #[test]
    fn minute_of_day_carries_into_wrapping_day_counter() {
        let mut rtc = HuC3Rtc::new();
        rtc.set_live_minutes(1_439);
        rtc.set_live_days(u16::MAX);
        rtc.advance_minutes(1);

        assert_eq!(rtc.live_minutes(), 0);
        assert_eq!(rtc.live_days(), 0);
    }

    #[test]
    fn atomic_transfer_rejects_invalid_minutes() {
        let mut rtc = HuC3Rtc::new();
        rtc.set_live_minutes(123);
        encode_nibbles(1_440, &mut rtc.memory[0..3]);

        assert!(!rtc.copy_transfer_to_current());
        assert_eq!(rtc.live_minutes(), 123);
    }

    #[test]
    fn persistent_sync_keeps_subminute_remainder_and_runs_once() {
        let saved_at = UNIX_EPOCH + Duration::from_secs(100);
        let mut rtc = HuC3Rtc::from_state(vec![0; RTC_MEMORY_SIZE], 0, Some(100)).unwrap();

        rtc.sync(saved_at + Duration::from_secs(125));
        rtc.sync(saved_at + Duration::from_secs(180));

        assert_eq!(rtc.live_minutes(), 2);
        assert_eq!(rtc.subminute_clocks, 5 * 4_194_304);
    }

    #[test]
    fn backward_clock_does_not_reverse_time() {
        let mut rtc = HuC3Rtc::from_state(vec![0; RTC_MEMORY_SIZE], 0, Some(100)).unwrap();
        rtc.sync(UNIX_EPOCH + Duration::from_secs(90));
        assert_eq!(rtc.live_minutes(), 0);
    }
}
