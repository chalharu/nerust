#[derive(serde::Serialize, serde::Deserialize, Debug, Copy, Clone)]
pub(crate) struct TimerDao {
    value: u16,
    period: u16,
}

impl TimerDao {
    pub(crate) fn new() -> Self {
        Self {
            value: 0,
            period: 0,
        }
    }

    pub(crate) fn reset(&mut self) {
        self.value = 0;
        self.period = 0;
    }

    pub(crate) fn set_period(&mut self, period: u16) {
        self.period = period;
    }

    pub(crate) fn set_value(&mut self, value: u16) {
        self.value = value;
    }

    pub(crate) fn step_timer(&mut self) -> bool {
        if self.value == 0 {
            self.value = self.period;
            true
        } else {
            self.value -= 1;
            false
        }
    }

    pub(crate) fn advance(&mut self, cycles: u64) -> u64 {
        if cycles == 0 {
            return 0;
        }

        if self.period == 0 {
            self.value = 0;
            return cycles;
        }

        let first_clock = u64::from(self.value) + 1;
        if cycles < first_clock {
            self.value -= cycles as u16;
            return 0;
        }

        let period = u64::from(self.period) + 1;
        let remaining = cycles - first_clock;
        let clocks = 1 + remaining / period;
        self.value = (u64::from(self.period) - (remaining % period)) as u16;
        clocks
    }

    pub(crate) fn value(&self) -> u16 {
        self.value
    }

    pub(crate) fn period(&self) -> u16 {
        self.period
    }

    pub(crate) fn get_period(&mut self) -> u16 {
        self.period
    }
}
