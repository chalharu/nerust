// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

#[cfg(test)]
mod audio_regression_test;
mod dmc;
pub(crate) mod envelope;
#[cfg(test)]
mod fft_test;
mod frame_counter;
pub(crate) mod length_counter;
mod noise;
mod pulse;
pub(crate) mod timer;
mod triangle;

use self::dmc::DMC;
use self::envelope::*;
use self::frame_counter::*;
use self::length_counter::*;
use self::noise::Noise;
use self::pulse::Pulse;
use self::triangle::Triangle;
use crate::Cpu;
use crate::OpenBusReadResult;
use crate::interrupt::{Interrupt, IrqSource};
use crate::persistence_error::PersistenceError;
use nerust_sound_traits::MixerInput;

// // 240Hz フレームシーケンサ
// const FRAME_COUNTER_RATE: f64 = 7457.3875;
// const FRAME_COUNTER_RATE: f64 = 29829.55;
const CLOCK_RATE: u64 = 1_789_773;

#[derive(serde_derive::Serialize, serde_derive::Deserialize, Debug, Clone)]
pub(crate) struct Core {
    #[serde(skip, default = "make_pulse_table")]
    pulse_table: Vec<f32>,
    #[serde(skip, default = "make_tnd_table")]
    tnd_table: Vec<f32>,
    // filter: ChaindFilter<ChaindFilter<PassFilter, PassFilter>, PassFilter>,
    // sample_rate: u32,
    pulse1: Pulse,
    pulse2: Pulse,
    triangle: Triangle,
    noise: Noise,
    dmc: DMC,
    // Channel timers stay CPU-cycle accurate; only exported mixer samples are rate-limited.
    #[serde(default)]
    sample_accumulator: u64,
    frame_counter: FrameCounter,
}

fn make_pulse_table() -> Vec<f32> {
    (0..31)
        .map(|x| 95.52 / (8128.0 / x as f32 + 100.0))
        .collect::<Vec<_>>()
}

fn make_tnd_table() -> Vec<f32> {
    (0..203)
        .map(|x| 163.67 / (24329.0 / x as f32 + 100.0))
        .collect::<Vec<_>>()
}

impl Core {
    pub(crate) fn new(
        // sample_rate: u32,
        interrupt: &mut Interrupt,
    ) -> Self {
        // let sample_reset_cycle = CLOCK_RATE * sample_rate as u64;
        // let filter_sample_rate = CLOCK_RATE as f64 / f64::from(sample_rate);
        let mut result = Self {
            // https://wiki.nesdev.com/w/index.php/APU_Mixer
            pulse_table: make_pulse_table(),
            tnd_table: make_tnd_table(),
            // filter: PassFilter::get_highpass_filter(filter_sample_rate, 90.0)
            //     .chain(PassFilter::get_highpass_filter(filter_sample_rate, 440.0))
            //     .chain(PassFilter::get_lowpass_filter(filter_sample_rate, 14000.0)),
            // sample_rate,
            pulse1: Pulse::new(true),
            pulse2: Pulse::new(false),
            triangle: Triangle::new(),
            noise: Noise::new(),
            dmc: DMC::new(),
            sample_accumulator: 0,
            frame_counter: FrameCounter::new(),
        };
        result.initialize(interrupt);
        result
    }

    pub(crate) fn validate_runtime_state(&self) -> Result<(), PersistenceError> {
        self.pulse1.validate_runtime_state()?;
        self.pulse2.validate_runtime_state()?;
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn set_pulse_duty_for_test(&mut self, mode: u8, value: u8) {
        self.pulse1.set_duty_for_test(mode, value);
    }

    pub(crate) fn reset(&mut self, interrupt: &mut Interrupt) {
        self.pulse1.reset();
        self.pulse2.reset();
        self.triangle.reset();
        self.noise.reset();
        self.dmc.reset();
        self.sample_accumulator = 0;
        self.frame_counter.reset();
        self.initialize(interrupt);
    }

    fn initialize(&mut self, interrupt: &mut Interrupt) {
        for _ in 0..4 {
            self.step_frame(interrupt);
        }
    }

    pub(crate) fn read_register(
        &mut self,
        address: usize,
        interrupt: &mut Interrupt,
    ) -> OpenBusReadResult {
        match address {
            0x4015 => OpenBusReadResult::new(self.read_status(interrupt), !0x20),
            _ => {
                log::error!("unhandled apu register read at address: 0x{:04X}", address);
                OpenBusReadResult::new(0, 0)
            }
        }
    }

    pub(crate) fn write_register(&mut self, address: usize, value: u8, interrupt: &mut Interrupt) {
        match address {
            0x4000 => self.pulse1.write_control(value),
            0x4001 => self.pulse1.write_sweep(value),
            0x4002 => self.pulse1.write_timer_low(value),
            0x4003 => self.pulse1.write_timer_high(value),
            0x4004 => self.pulse2.write_control(value),
            0x4005 => self.pulse2.write_sweep(value),
            0x4006 => self.pulse2.write_timer_low(value),
            0x4007 => self.pulse2.write_timer_high(value),
            0x4008 => self.triangle.write_control(value),
            0x4009 => (),
            0x4010 => self.dmc.write_control(value, interrupt),
            0x4011 => self.dmc.write_value(value),
            0x4012 => self.dmc.write_address(value),
            0x4013 => self.dmc.write_length(value),
            0x400A => self.triangle.write_timer_low(value),
            0x400B => self.triangle.write_timer_high(value),
            0x400C => self.noise.write_control(value),
            0x400D => (),
            0x400E => self.noise.write_period(value),
            0x400F => self.noise.write_length(value),
            0x4015 => self.write_control(value, interrupt),
            0x4017 => self.frame_counter.write_frame_counter(value, interrupt),
            _ => log::error!("unhandled apu register write at address: 0x{:04X}", address),
        }
    }

    pub(crate) fn dmc_fill(&mut self, value: u8, interrupt: &mut Interrupt) {
        self.dmc.fill(value, interrupt);
    }

    pub(crate) fn dmc_fill_address(&self) -> Option<usize> {
        self.dmc.fill_address()
    }

    fn step_frame(&mut self, interrupt: &mut Interrupt) {
        match self.frame_counter.step_frame_counter(interrupt) {
            FrameType::Half => {
                self.quarter_frame();
                self.half_frame();
            }
            FrameType::Quarter => self.quarter_frame(),
            FrameType::None => (),
        }
        self.step_timer(interrupt);
    }

    pub(crate) fn step<M: MixerInput>(
        &mut self,
        cpu: &mut Cpu,
        mixer: &mut M,
        mixer_sample_rate: u32,
        expansion_audio_output: f32,
        expansion_audio_inverted: bool,
    ) {
        self.step_frame(cpu.interrupt_mut());
        if self.sample_accumulator >= CLOCK_RATE {
            self.sample_accumulator %= CLOCK_RATE;
        }
        self.sample_accumulator += u64::from(mixer_sample_rate).min(CLOCK_RATE);
        if self.sample_accumulator >= CLOCK_RATE {
            self.sample_accumulator -= CLOCK_RATE;
            self.send_sample(mixer, expansion_audio_output, expansion_audio_inverted);
        }
    }

    pub(crate) fn step_many<M: MixerInput>(
        &mut self,
        cpu: &mut Cpu,
        mixer: &mut M,
        mixer_sample_rate: u32,
        expansion_audio_output: f32,
        expansion_audio_inverted: bool,
        cycles: u64,
    ) {
        for _ in 0..cycles {
            self.step_frame(cpu.interrupt_mut());
            if self.sample_accumulator >= CLOCK_RATE {
                self.sample_accumulator %= CLOCK_RATE;
            }
            self.sample_accumulator += u64::from(mixer_sample_rate).min(CLOCK_RATE);
            if self.sample_accumulator >= CLOCK_RATE {
                self.sample_accumulator -= CLOCK_RATE;
                self.send_sample(mixer, expansion_audio_output, expansion_audio_inverted);
            }
        }
    }

    pub(crate) fn cycles_until_next_scheduler_event(
        &self,
        interrupt: &Interrupt,
        max_cycles: u64,
    ) -> u64 {
        self.frame_counter
            .cycles_until_next_irq_change(interrupt, max_cycles)
            .min(self.dmc.cycles_until_next_dma_request(max_cycles))
    }

    pub(crate) fn send_sample<M: MixerInput>(
        &self,
        mixer: &mut M,
        expansion_audio_output: f32,
        expansion_audio_inverted: bool,
    ) {
        mixer.push(self.output(expansion_audio_output, expansion_audio_inverted));
        // let output = self.output();
        // let filtered = self.filter.step(output);
        // speaker.push(((filtered * 65535.0) as i32 - 32768) as i16);
    }

    pub(crate) fn output(
        &self,
        expansion_audio_output: f32,
        expansion_audio_inverted: bool,
    ) -> f32 {
        let apu_output = self.pulse_table
            [usize::from(self.pulse1.output()) + usize::from(self.pulse2.output())]
            + self.tnd_table[3 * usize::from(self.triangle.output())
                + 2 * usize::from(self.noise.output())
                + usize::from(self.dmc.output())];
        if expansion_audio_inverted {
            (0.5 + (apu_output - expansion_audio_output) * 0.5).clamp(0.0, 1.0)
        } else {
            (apu_output + expansion_audio_output).clamp(0.0, 1.0)
        }
    }

    fn quarter_frame(&mut self) {
        self.step_envelope();
    }

    fn half_frame(&mut self) {
        self.step_sweep();
        self.step_length();
    }

    fn step_timer(&mut self, interrupt: &mut Interrupt) {
        self.pulse1.step_length_counter();
        self.pulse2.step_length_counter();
        self.noise.step_length_counter();
        self.pulse1.step_timer();
        self.pulse2.step_timer();
        self.noise.step_timer();
        self.dmc.step_timer(interrupt);
        self.triangle.step_timer();
    }

    fn step_envelope(&mut self) {
        self.pulse1.step_envelope();
        self.pulse2.step_envelope();
        self.noise.step_envelope();
        self.triangle.step_counter();
    }

    fn step_sweep(&mut self) {
        self.pulse1.step_sweep();
        self.pulse2.step_sweep();
    }

    fn step_length(&mut self) {
        self.pulse1.step_length();
        self.pulse2.step_length();
        self.noise.step_length();
        self.triangle.step_length();
    }

    fn read_status(&mut self, interrupt: &mut Interrupt) -> u8 {
        let result = (if self.pulse1.get_status() { 1 } else { 0 })
            | (if self.pulse2.get_status() { 2 } else { 0 })
            | (if self.triangle.get_status() { 4 } else { 0 })
            | (if self.noise.get_status() { 8 } else { 0 })
            | (if self.dmc.get_status() { 0x10 } else { 0 })
            | (if interrupt.get_irq(IrqSource::FRAME_COUNTER) {
                0x40
            } else {
                0
            })
            | (if interrupt.get_irq(IrqSource::DMC) {
                0x80
            } else {
                0
            });
        interrupt.clear_irq(IrqSource::FRAME_COUNTER);
        result
    }

    fn write_control(&mut self, value: u8, interrupt: &mut Interrupt) {
        self.pulse1.set_enabled((value & 1) != 0);
        self.pulse2.set_enabled((value & 2) != 0);
        self.triangle.set_enabled((value & 4) != 0);
        self.noise.set_enabled((value & 8) != 0);
        self.dmc.set_enabled((value & 16) != 0, interrupt);
    }
}

#[cfg(test)]
mod tests {
    use super::Core;
    use crate::interrupt::Interrupt;

    #[test]
    fn inverted_expansion_audio_mix_centers_silence_and_flips_contribution() {
        let mut interrupt = Interrupt::new();
        let apu = Core::new(&mut interrupt);

        assert!((apu.output(0.0, false) - 0.0).abs() < f32::EPSILON);
        assert!((apu.output(0.0, true) - 0.5).abs() < f32::EPSILON);
        assert!(apu.output(0.25, true) < 0.5);
    }
}
