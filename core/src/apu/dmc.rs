// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use super::timer::*;
use crate::cpu::interrupt::*;

// NTSC
// https://wiki.nesdev.com/w/index.php/APU_DMC
// 2で1APUサイクル
const DMC_TABLE: [u8; 16] = [
    214, 190, 170, 160, 143, 127, 113, 107, 95, 80, 71, 64, 53, 42, 36, 27,
];

#[derive(serde_derive::Serialize, serde_derive::Deserialize, Debug, Copy, Clone)]
#[allow(
    clippy::upper_case_acronyms,
    reason = "DMC is the established NES APU channel name"
)]
pub(crate) struct DMC {
    value: u8,

    sample_address: u16,
    sample_length: u16,
    length_value: u16,
    current_address: u16,
    shift_register: u8,
    bit_count: u8,
    read_buffer: u8,

    enabled: bool,
    need_buffer: bool,
    is_loop: bool,
    irq: bool,
    timer: TimerDao,
}

impl HaveTimerDao for DMC {
    fn timer_dao(&self) -> &TimerDao {
        &self.timer
    }
    fn timer_dao_mut(&mut self) -> &mut TimerDao {
        &mut self.timer
    }
}

impl DMC {
    pub(crate) fn new() -> Self {
        Self {
            shift_register: 0,
            bit_count: 0,
            enabled: false,
            need_buffer: true,
            current_address: 0,
            read_buffer: 0,
            value: 0,
            length_value: 0,
            sample_address: 0,
            sample_length: 0,
            is_loop: false,
            irq: false,
            timer: TimerDao::new(),
        }
    }

    pub(crate) fn reset(&mut self) {
        self.timer.reset();
        let period = (u16::from(DMC_TABLE[0]) << 1) - 1;
        self.timer.set_period(period);
        self.timer.set_value(period);
        self.is_loop = false;
        self.irq = false;
    }

    pub(crate) fn write_control(&mut self, value: u8, interrupt: &mut Interrupt) {
        self.irq = (value & 0x80) != 0;
        self.is_loop = (value & 0x40) != 0;
        self.timer
            .set_period((u16::from(DMC_TABLE[usize::from(value & 0x0f)]) << 1) - 1);
        if !self.irq {
            interrupt.clear_irq(IrqSource::DMC);
        }
    }

    pub(crate) fn write_value(&mut self, value: u8) {
        self.value = value & 0x7F;
    }

    pub(crate) fn write_address(&mut self, value: u8) {
        self.sample_address = 0xC000 | (u16::from(value) << 6);
    }

    pub(crate) fn write_length(&mut self, value: u8) {
        self.sample_length = 1 | (u16::from(value) << 4);
    }

    pub(crate) fn set_enabled(&mut self, enabled: bool, interrupt: &mut Interrupt) {
        if !enabled {
            self.length_value = 0;
        } else if self.length_value == 0 {
            self.restart();
            if self.need_buffer && self.length_value > 0 {
                interrupt.dmc_dma_request = Some(DmcDmaKind::Load);
            }
        }
    }

    pub(crate) fn restart(&mut self) {
        self.current_address = self.sample_address;
        self.length_value = self.sample_length;
    }

    pub(crate) fn fill_address(&self) -> Option<usize> {
        if self.get_status() {
            Some(self.current_address as usize)
        } else {
            None
        }
    }

    pub(crate) fn get_status(&self) -> bool {
        self.length_value > 0
    }

    pub(crate) fn fill(&mut self, value: u8, interrupt: &mut Interrupt) {
        if self.length_value > 0 {
            self.read_buffer = value;
            self.need_buffer = false;

            self.current_address = self.current_address.wrapping_add(1);
            self.length_value -= 1;

            // if self.current_address == 0 {
            //     self.current_address = 0x8000;
            // }
            if self.length_value == 0 {
                if self.is_loop {
                    self.restart();
                } else if self.irq {
                    interrupt.set_irq(IrqSource::DMC);
                }
            }
        }
    }

    pub(crate) fn step_timer(&mut self, interrupt: &mut Interrupt) {
        if self.timer.step_timer() {
            if self.enabled {
                self.step_shifter();
            }
            if self.bit_count > 0 {
                self.bit_count -= 1;
            }

            self.step_reader(interrupt);
        }
    }

    pub(crate) fn step_reader(&mut self, interrupt: &mut Interrupt) {
        if self.bit_count == 0 {
            self.bit_count = 8;
            if self.need_buffer {
                self.enabled = false;
            } else {
                self.enabled = true;
                self.shift_register = self.read_buffer;
                self.need_buffer = true;
                if self.length_value > 0 {
                    interrupt.dmc_dma_request = Some(DmcDmaKind::Reload);
                }
            }
        }
    }

    pub(crate) fn step_shifter(&mut self) {
        if (self.shift_register & 1) != 0 {
            if self.value <= 125 {
                self.value += 2;
            }
        } else {
            self.value = self.value.saturating_sub(2);
        }

        self.shift_register >>= 1;
    }

    pub(crate) fn output(&self) -> u8 {
        self.value
    }
}

#[cfg(test)]
mod tests {
    use super::super::fft_test::{
        CPU_CLOCK_HZ, FFT_SAMPLE_COUNT, capture_samples, dominant_frequency,
        dominant_frequency_tolerance,
    };
    use super::{DMC, DMC_TABLE};
    use crate::cpu::interrupt::Interrupt;

    fn expected_single_sample_frequency(rate_index: usize) -> f32 {
        CPU_CLOCK_HZ / (16.0 * f32::from(DMC_TABLE[rate_index]))
    }

    fn test_single_sample_dmc(rate_index: u8, sample_byte: u8) -> (DMC, Interrupt) {
        let mut interrupt = Interrupt::new();
        let mut dmc = DMC::new();
        dmc.reset();
        dmc.write_control(0x40 | rate_index, &mut interrupt);
        dmc.write_value(64);
        dmc.write_length(0);
        dmc.set_enabled(true, &mut interrupt);
        if interrupt.dmc_dma_request.take().is_some() {
            dmc.fill(sample_byte, &mut interrupt);
        }
        dmc.step_reader(&mut interrupt);
        if interrupt.dmc_dma_request.take().is_some() {
            dmc.fill(sample_byte, &mut interrupt);
        }
        (dmc, interrupt)
    }

    fn step_single_sample_dmc(dmc: &mut DMC, interrupt: &mut Interrupt, sample_byte: u8) -> f32 {
        dmc.step_timer(interrupt);
        if interrupt.dmc_dma_request.take().is_some() {
            dmc.fill(sample_byte, interrupt);
        }
        f32::from(dmc.output())
    }

    #[test]
    fn step_shifter_clamps_output_at_zero() {
        let mut dmc = DMC::new();
        dmc.write_value(0);
        dmc.step_shifter();

        assert_eq!(dmc.output(), 0);
    }

    #[test]
    fn step_shifter_increases_output_by_two_for_set_bit() {
        let mut dmc = DMC::new();
        dmc.write_value(4);
        dmc.shift_register = 1;
        dmc.step_shifter();

        assert_eq!(dmc.output(), 6);
    }

    #[test]
    fn write_value_updates_output_immediately_without_smoothing() {
        let mut dmc = DMC::new();

        dmc.write_value(0x7F);
        assert_eq!(dmc.output(), 0x7F);

        dmc.write_value(0);
        assert_eq!(dmc.output(), 0);
    }

    #[test]
    fn write_value_masks_to_seven_bits() {
        let mut dmc = DMC::new();

        dmc.write_value(0xFF);

        assert_eq!(dmc.output(), 0x7F);
    }

    #[test]
    fn fft_peak_matches_expected_single_sample_frequency() {
        let rate_index = 11_usize;
        let sample_byte = 0xF0;
        let (mut dmc, mut interrupt) = test_single_sample_dmc(rate_index as u8, sample_byte);
        let samples = capture_samples(FFT_SAMPLE_COUNT, || {
            step_single_sample_dmc(&mut dmc, &mut interrupt, sample_byte)
        });
        let dominant = dominant_frequency(&samples, CPU_CLOCK_HZ);

        assert!(
            (dominant - expected_single_sample_frequency(rate_index)).abs()
                <= dominant_frequency_tolerance(CPU_CLOCK_HZ, FFT_SAMPLE_COUNT)
        );
    }
}
