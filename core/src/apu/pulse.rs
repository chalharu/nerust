// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use super::envelope::*;
use super::length_counter::*;
use super::timer::*;
use crate::persistence_error::PersistenceError;

const DUTY_TABLE: [[bool; 8]; 4] = [
    [false, true, false, false, false, false, false, false],
    [false, true, true, false, false, false, false, false],
    [false, true, true, true, true, false, false, false],
    [true, false, false, true, true, true, true, true],
];

#[derive(serde_derive::Serialize, serde_derive::Deserialize, Debug, Copy, Clone)]
pub(crate) struct Pulse {
    is_first_channel: bool,
    duty_mode: u8,
    duty_value: u8,
    sweep_reload: bool,
    sweep_enabled: bool,
    sweep_negate: bool,
    sweep_shift: u8,
    sweep_period: u8,
    sweep_value: u8,
    sweep_target_period: u16,
    period: u16,
    envelope: EnvelopeDao,
    length_counter: LengthCounterDao,
    timer: TimerDao,
}

impl HaveLengthCounterDao for Pulse {
    fn length_counter_dao(&self) -> &LengthCounterDao {
        &self.length_counter
    }
    fn length_counter_dao_mut(&mut self) -> &mut LengthCounterDao {
        &mut self.length_counter
    }
}

impl HaveEnvelopeDao for Pulse {
    fn envelope_dao(&self) -> &EnvelopeDao {
        &self.envelope
    }
    fn envelope_dao_mut(&mut self) -> &mut EnvelopeDao {
        &mut self.envelope
    }
}

impl HaveLengthCounter for Pulse {
    type LengthCounter = Self;
    fn length_counter(&self) -> &Self::LengthCounter {
        self
    }
    fn length_counter_mut(&mut self) -> &mut Self::LengthCounter {
        self
    }
}

impl HaveTimerDao for Pulse {
    fn timer_dao(&self) -> &TimerDao {
        &self.timer
    }
    fn timer_dao_mut(&mut self) -> &mut TimerDao {
        &mut self.timer
    }
}

impl Pulse {
    pub(crate) fn new(is_first_channel: bool) -> Self {
        Self {
            is_first_channel,
            duty_mode: 0,
            duty_value: 0,
            period: 0,
            sweep_reload: false,
            sweep_enabled: false,
            sweep_negate: false,
            sweep_shift: 0,
            sweep_period: 0,
            sweep_value: 0,
            sweep_target_period: 0,
            envelope: EnvelopeDao::new(),
            length_counter: LengthCounterDao::new(),
            timer: TimerDao::new(),
        }
    }

    pub(crate) fn reset(&mut self) {
        self.length_counter.reset();
        self.envelope.reset();
        self.timer.reset();

        self.duty_mode = 0;
        self.duty_value = 0;
        self.period = 0;
        self.sweep_enabled = false;
        self.sweep_period = 0;
        self.sweep_negate = false;
        self.sweep_shift = 0;
        self.sweep_reload = false;
        self.sweep_value = 0;
        self.sweep_target_period = 0;
        self.sweep();
    }

    pub(crate) fn write_control(&mut self, value: u8) {
        self.length_counter.set_halt((value & 0x20) != 0);
        self.envelope.set_enabled((value & 0x10) == 0);
        self.envelope.set_period(value & 0x0F);
        self.duty_mode = (value >> 6) & 3;
    }

    pub(crate) fn write_sweep(&mut self, value: u8) {
        self.sweep_enabled = (value & 0x80) != 0;
        self.sweep_period = ((value >> 4) & 7) + 1;
        self.sweep_negate = (value & 0x08) != 0;
        self.sweep_shift = value & 7;
        self.sweep_reload = true;
        self.sweep();
    }

    fn set_period(&mut self, period: u16) {
        self.period = period;
        self.timer.set_period((period << 1) + 1);
        self.sweep();
    }

    pub(crate) fn write_timer_low(&mut self, value: u8) {
        self.set_period((self.period & 0xFF00) | u16::from(value));
    }

    pub(crate) fn write_timer_high(&mut self, value: u8) {
        self.length_counter.set_load(value >> 3);
        self.set_period((self.period & 0xFF) | (u16::from(value & 7) << 8));
        self.duty_value = 0;
        self.envelope.restart();
    }

    pub(crate) fn step_timer(&mut self) {
        if self.timer.step_timer() {
            self.duty_value = self.duty_value.wrapping_sub(1) & 7;
        }
    }

    pub(crate) fn step_sweep(&mut self) {
        let divider_expired = self.sweep_value == 0;
        if divider_expired
            && self.sweep_enabled
            && self.sweep_shift > 0
            && self.period >= 8
            && self.sweep_target_period <= 0x7FF
        {
            self.set_period(self.sweep_target_period);
        }

        if divider_expired || self.sweep_reload {
            self.sweep_value = self.sweep_period;
            self.sweep_reload = false;
        } else {
            self.sweep_value -= 1;
        }
    }

    fn sweep(&mut self) {
        let delta = self.period >> self.sweep_shift;
        self.sweep_target_period = if self.sweep_negate {
            self.period
                .saturating_sub(delta + if self.is_first_channel { 1 } else { 0 })
        } else {
            self.period + delta
        }
    }

    pub(crate) fn output(&self) -> u8 {
        if (self.period < 8 || (!self.sweep_negate && self.sweep_target_period > 0x7FF))
            || !DUTY_TABLE[usize::from(self.duty_mode)][usize::from(self.duty_value)]
        {
            0
        } else {
            Envelope::get_volume(self)
        }
    }

    pub(crate) fn validate_runtime_state(&self) -> Result<(), PersistenceError> {
        if usize::from(self.duty_mode) >= DUTY_TABLE.len() {
            return Err(PersistenceError::Validation(
                "APU pulse duty mode overflow".into(),
            ));
        }
        if usize::from(self.duty_value) >= DUTY_TABLE[0].len() {
            return Err(PersistenceError::Validation(
                "APU pulse duty value overflow".into(),
            ));
        }
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn set_duty_for_test(&mut self, mode: u8, value: u8) {
        self.duty_mode = mode;
        self.duty_value = value;
    }
}

#[cfg(test)]
mod tests {
    use super::super::fft_test::{
        CPU_CLOCK_HZ, FFT_SAMPLE_COUNT, capture_samples, dominant_frequency,
        dominant_frequency_tolerance, peak_power_near_frequency, power_spectrum,
    };
    use super::Pulse;

    fn expected_frequency(raw_period: u16) -> f32 {
        CPU_CLOCK_HZ / (16.0 * (f32::from(raw_period) + 1.0))
    }

    fn test_fixed_pulse(is_first_channel: bool, raw_period: u16) -> Pulse {
        let mut pulse = Pulse::new(is_first_channel);
        pulse.write_control(0xBF);
        pulse.length_counter.set_enabled(true);
        pulse.write_timer_low(raw_period as u8);
        pulse.write_timer_high(((raw_period >> 8) as u8 & 0x07) | 0xF8);
        pulse.length_counter.step();
        pulse
    }

    #[test]
    fn step_sweep_applies_target_period_when_divider_expires() {
        let mut pulse = Pulse::new(true);
        pulse.set_period(0x0100);
        pulse.write_sweep(0b1000_0001);

        pulse.step_sweep();
        assert_eq!(pulse.period, 0x0180);
        assert_eq!(pulse.timer.get_period(), 0x0301);
        assert_eq!(pulse.sweep_target_period, 0x0240);
        assert_eq!(pulse.sweep_value, pulse.sweep_period);
    }

    #[test]
    fn step_sweep_preserves_negate_difference_between_pulse_channels() {
        let mut pulse1 = Pulse::new(true);
        pulse1.set_period(0x0020);
        pulse1.write_sweep(0b1000_1001);
        pulse1.step_sweep();

        let mut pulse2 = Pulse::new(false);
        pulse2.set_period(0x0020);
        pulse2.write_sweep(0b1000_1001);
        pulse2.step_sweep();

        assert_eq!(pulse1.period, 0x000F);
        assert_eq!(pulse2.period, 0x0010);
    }

    #[test]
    fn step_sweep_reload_delays_update_until_divider_expires_when_nonzero() {
        let mut pulse = Pulse::new(true);
        pulse.set_period(0x0100);
        pulse.sweep_value = 1;
        pulse.write_sweep(0b1000_0010);

        pulse.step_sweep();
        assert_eq!(pulse.period, 0x0100);
        assert_eq!(pulse.sweep_value, pulse.sweep_period);

        pulse.step_sweep();
        assert_eq!(pulse.period, 0x0100);

        pulse.step_sweep();
        assert_eq!(pulse.period, 0x0140);
    }

    #[test]
    fn fft_peak_matches_expected_fixed_pulse_frequency() {
        let mut pulse = test_fixed_pulse(true, 0x0020);
        let samples = capture_samples(FFT_SAMPLE_COUNT, || {
            pulse.step_timer();
            f32::from(pulse.output())
        });
        let dominant = dominant_frequency(&samples, CPU_CLOCK_HZ);

        assert!(
            (dominant - expected_frequency(0x0020)).abs()
                <= dominant_frequency_tolerance(CPU_CLOCK_HZ, FFT_SAMPLE_COUNT)
        );
    }

    #[test]
    fn fft_fixed_pulse_keeps_expected_odd_harmonic_profile() {
        let raw_period = 0x0020;
        let mut pulse = test_fixed_pulse(true, raw_period);
        let samples = capture_samples(FFT_SAMPLE_COUNT, || {
            pulse.step_timer();
            f32::from(pulse.output())
        });
        let spectrum = power_spectrum(&samples);
        let fundamental = expected_frequency(raw_period);
        let first = peak_power_near_frequency(&spectrum, CPU_CLOCK_HZ, fundamental, 2);
        let second = peak_power_near_frequency(&spectrum, CPU_CLOCK_HZ, fundamental * 2.0, 2);
        let third = peak_power_near_frequency(&spectrum, CPU_CLOCK_HZ, fundamental * 3.0, 2);
        let fourth = peak_power_near_frequency(&spectrum, CPU_CLOCK_HZ, fundamental * 4.0, 2);
        let fifth = peak_power_near_frequency(&spectrum, CPU_CLOCK_HZ, fundamental * 5.0, 2);

        assert!(first > third * 5.0);
        assert!(third > fifth * 2.0);
        assert!(third > first * 0.07);
        assert!(fifth > first * 0.025);
        assert!(first > second * 1_000.0);
        assert!(third > second * 100.0);
        assert!(fifth > fourth * 100.0);
    }

    #[test]
    fn fft_peak_moves_after_sweep_updates_period() {
        let mut pulse = test_fixed_pulse(true, 0x0040);
        let before = dominant_frequency(
            &capture_samples(FFT_SAMPLE_COUNT, || {
                pulse.step_timer();
                f32::from(pulse.output())
            }),
            CPU_CLOCK_HZ,
        );

        pulse.write_sweep(0b1000_1001);
        pulse.step_sweep();

        let after = dominant_frequency(
            &capture_samples(FFT_SAMPLE_COUNT, || {
                pulse.step_timer();
                f32::from(pulse.output())
            }),
            CPU_CLOCK_HZ,
        );
        let tolerance = dominant_frequency_tolerance(CPU_CLOCK_HZ, FFT_SAMPLE_COUNT);

        assert!((before - expected_frequency(0x0040)).abs() <= tolerance);
        assert!((after - expected_frequency(0x001F)).abs() <= tolerance);
        assert!(after > before * 1.8);
    }
}
