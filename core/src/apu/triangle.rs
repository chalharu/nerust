// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use super::length_counter::*;
use super::timer::*;

const TRIANGLE_TABLE: [u8; 32] = [
    15, 14, 13, 12, 11, 10, 9, 8, 7, 6, 5, 4, 3, 2, 1, 0, 0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12,
    13, 14, 15,
];

#[derive(serde_derive::Serialize, serde_derive::Deserialize, Debug, Copy, Clone)]
pub(crate) struct Triangle {
    duty_value: u8,
    counter_period: u8,
    counter_value: u8,
    counter_reload: bool,
    counter_control: bool,
    output_value: u8,

    length_counter: LengthCounterDao,
    timer: TimerDao,
}

impl HaveLengthCounterDao for Triangle {
    fn length_counter_dao(&self) -> &LengthCounterDao {
        &self.length_counter
    }
    fn length_counter_dao_mut(&mut self) -> &mut LengthCounterDao {
        &mut self.length_counter
    }
}

impl HaveTimerDao for Triangle {
    fn timer_dao(&self) -> &TimerDao {
        &self.timer
    }
    fn timer_dao_mut(&mut self) -> &mut TimerDao {
        &mut self.timer
    }
}

impl Triangle {
    pub(crate) fn new() -> Self {
        Self {
            duty_value: 0,
            counter_reload: false,
            counter_control: false,
            counter_period: 0,
            counter_value: 0,
            length_counter: LengthCounterDao::new(),
            timer: TimerDao::new(),
            output_value: 0,
        }
    }

    pub(crate) fn reset(&mut self) {
        self.length_counter.soft_reset();
        self.timer.reset();
        self.duty_value = 0;
        self.counter_reload = false;
        self.counter_control = false;
        self.counter_period = 0;
        self.counter_value = 0;
    }

    pub(crate) fn write_control(&mut self, value: u8) {
        self.counter_control = (value & 0x80) != 0;
        self.counter_period = value & 0x7F;
        self.length_counter.set_halt(self.counter_control);
    }

    pub(crate) fn write_timer_low(&mut self, value: u8) {
        let period = self.timer.get_period();
        self.timer.set_period((period & 0xFF00) | u16::from(value));
    }

    pub(crate) fn write_timer_high(&mut self, value: u8) {
        self.length_counter.set_load(value >> 3);
        let period = self.timer.get_period();
        self.timer
            .set_period((period & 0xFF) | (u16::from(value & 7) << 8));
        self.counter_reload = true;
    }

    pub(crate) fn step_timer(&mut self) {
        if self.timer.step_timer() && self.length_counter.get_status() && self.counter_value > 0 {
            self.duty_value = (self.duty_value + 1) & 0x1F;
            if self.timer.get_period() > 1 {
                self.output_value = TRIANGLE_TABLE[usize::from(self.duty_value)];
            }
        }
    }

    pub(crate) fn step_timer_many(&mut self, cycles: u64) {
        let clocks = self.timer.advance(cycles);
        if clocks == 0 || !self.length_counter.get_status() || self.counter_value == 0 {
            return;
        }

        self.duty_value = ((u64::from(self.duty_value) + clocks) & 0x1F) as u8;
        if self.timer.get_period() > 1 {
            self.output_value = TRIANGLE_TABLE[usize::from(self.duty_value)];
        }
    }

    pub(crate) fn step_counter(&mut self) {
        if self.counter_reload {
            self.counter_value = self.counter_period;
        } else if self.counter_value > 0 {
            self.counter_value -= 1;
        }
        if self.counter_control {
            self.counter_reload = false;
        }
    }

    pub(crate) fn output(&self) -> u8 {
        self.output_value
    }
}

#[cfg(test)]
mod tests {
    use super::super::fft_test::{
        CPU_CLOCK_HZ, FFT_SAMPLE_COUNT, capture_samples, dominant_frequency,
        dominant_frequency_tolerance,
    };
    use super::Triangle;

    fn expected_frequency(raw_period: u16) -> f32 {
        CPU_CLOCK_HZ / (32.0 * (f32::from(raw_period) + 1.0))
    }

    fn test_triangle(raw_period: u16) -> Triangle {
        let mut triangle = Triangle::new();
        triangle.write_control(0xFF);
        triangle.length_counter.set_enabled(true);
        triangle.write_timer_low(raw_period as u8);
        triangle.write_timer_high(((raw_period >> 8) as u8 & 0x07) | 0xF8);
        triangle.length_counter.step();
        triangle.step_counter();
        triangle
    }

    #[test]
    fn fft_peak_matches_expected_triangle_frequency() {
        let mut triangle = test_triangle(0x0010);
        let samples = capture_samples(FFT_SAMPLE_COUNT, || {
            triangle.step_timer();
            f32::from(triangle.output())
        });
        let dominant = dominant_frequency(&samples, CPU_CLOCK_HZ);

        assert!(
            (dominant - expected_frequency(0x0010)).abs()
                <= dominant_frequency_tolerance(CPU_CLOCK_HZ, FFT_SAMPLE_COUNT)
        );
    }

    #[test]
    fn fft_peak_moves_after_triangle_period_change() {
        let mut triangle = test_triangle(0x0040);
        let before = dominant_frequency(
            &capture_samples(FFT_SAMPLE_COUNT, || {
                triangle.step_timer();
                f32::from(triangle.output())
            }),
            CPU_CLOCK_HZ,
        );

        triangle.write_timer_low(0x20);
        triangle.write_timer_high(0xF8);
        triangle.step_counter();

        let after = dominant_frequency(
            &capture_samples(FFT_SAMPLE_COUNT, || {
                triangle.step_timer();
                f32::from(triangle.output())
            }),
            CPU_CLOCK_HZ,
        );
        let tolerance = dominant_frequency_tolerance(CPU_CLOCK_HZ, FFT_SAMPLE_COUNT);

        assert!((before - expected_frequency(0x0040)).abs() <= tolerance);
        assert!((after - expected_frequency(0x0020)).abs() <= tolerance);
        assert!(after > before);
    }
}
