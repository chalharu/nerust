// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

pub(crate) struct TimerDao {
    value: u16,
    period: u16,
}

impl TimerDao {
    pub fn new() -> Self {
        Self {
            value: 0,
            period: 0,
        }
    }

    pub fn reset(&mut self) {
        self.value = 0;
        self.period = 0;
    }

    pub fn set_period(&mut self, period: u16) {
        self.period = period;
    }

    pub fn set_value(&mut self, value: u16) {
        self.value = value;
    }

    pub fn step_timer(&mut self) -> bool {
        if self.value == 0 {
            self.value = self.period;
            true
        } else {
            self.value -= 1;
            false
        }
    }

    pub fn get_period(&mut self) -> u16 {
        self.period
    }
}

pub(crate) trait HaveTimerDao {
    fn timer_dao(&self) -> &TimerDao;
    fn timer_dao_mut(&mut self) -> &mut TimerDao;
}

pub(crate) trait Timer: HaveTimerDao {
    fn reset(&mut self) {
        self.timer_dao_mut().reset()
    }

    fn set_period(&mut self, period: u16) {
        self.timer_dao_mut().set_period(period)
    }

    fn set_timer(&mut self, value: u16) {
        self.timer_dao_mut().set_value(value)
    }

    fn step_timer(&mut self) -> bool {
        self.timer_dao_mut().step_timer()
    }
}

impl<T: HaveTimerDao> Timer for T {}

pub(crate) trait HaveTimer {
    type Timer: Timer;
    fn timer(&self) -> &Self::Timer;
    fn timer_mut(&mut self) -> &mut Self::Timer;
}
