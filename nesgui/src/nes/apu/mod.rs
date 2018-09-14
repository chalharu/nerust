// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

mod dmc;
mod filter;
mod noise;
mod pulse;
mod triangle;

use self::dmc::DMC;
use self::filter::*;
use self::noise::Noise;
use self::pulse::Pulse;
use self::triangle::Triangle;
use nes::cpu::interrupt::IrqReason;
use nes::cpu::State;
use nes::{Cartridge, Speaker};

// // 240Hz フレームシーケンサ
// const FRAME_COUNTER_RATE: f64 = 7457.3875;
// const FRAME_COUNTER_RATE: f64 = 29829.55;
const CLOCK_RATE: u64 = 1_789_773;
const LENGTH_TABLE: [u8; 32] = [
    0x0A, 0xFE, 0x14, 0x02, 0x28, 0x04, 0x50, 0x06, 0xA0, 0x08, 0x3C, 0x0A, 0x0E, 0x0C, 0x1A, 0x0E,
    0x0C, 0x10, 0x18, 0x12, 0x30, 0x14, 0x60, 0x16, 0xC0, 0x18, 0x48, 0x1A, 0x10, 0x1C, 0x20, 0x1E,
];

pub struct Core {
    pulse_table: Vec<f32>,
    tnd_table: Vec<f32>,
    filter: ChaindFilter<ChaindFilter<PassFilter, PassFilter>, PassFilter>,
    sample_rate: u32,
    pulse1: Pulse,
    pulse2: Pulse,
    triangle: Triangle,
    noise: Noise,
    dmc: DMC,
    cycle: u16,
    frame_period: bool,
    frame_irq: bool,
    sample_cycle: u64,
    sample_reset_cycle: u64,
    mode1_first: bool,
}

impl Core {
    pub fn new(sample_rate: u32) -> Self {
        let sample_reset_cycle = CLOCK_RATE * sample_rate as u64;
        let filter_sample_rate = CLOCK_RATE as f64 / f64::from(sample_rate);
        Self {
            // https://wiki.nesdev.com/w/index.php/APU_Mixer
            pulse_table: (0..31)
                .map(|x| 95.52 / (8128.0 / x as f32 + 100.0))
                .collect::<Vec<_>>(),
            tnd_table: (0..203)
                .map(|x| 163.67 / (24329.0 / x as f32 + 100.0))
                .collect::<Vec<_>>(),
            filter: PassFilter::get_highpass_filter(filter_sample_rate, 90.0)
                .chain(PassFilter::get_highpass_filter(filter_sample_rate, 440.0))
                .chain(PassFilter::get_lowpass_filter(filter_sample_rate, 14000.0)),
            sample_rate,
            pulse1: Pulse::new(true),
            pulse2: Pulse::new(false),
            triangle: Triangle::new(),
            noise: Noise::new(),
            dmc: DMC::new(),
            cycle: 2,
            frame_period: false,
            frame_irq: false,
            sample_cycle: 0,
            sample_reset_cycle,
            mode1_first: false,
        }
    }

    pub(crate) fn read_register(&mut self, address: usize, state: &mut State) -> u8 {
        match address {
            0x4015 => self.read_status(state),
            _ => {
                error!("unhandled apu register read at address: 0x{:04X}", address);
                0
            }
        }
    }

    pub(crate) fn write_register(&mut self, address: usize, value: u8, state: &mut State) {
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
            0x4010 => self.dmc.write_control(value, state),
            0x4011 => self.dmc.write_value(value),
            0x4012 => self.dmc.write_address(value),
            0x4013 => self.dmc.write_length(value),
            0x400A => self.triangle.write_timer_low(value),
            0x400B => self.triangle.write_timer_high(value),
            0x400C => self.noise.write_control(value),
            0x400D => (),
            0x400E => self.noise.write_period(value),
            0x400F => self.noise.write_length(value),
            0x4015 => self.write_control(value, state),
            0x4017 => self.write_frame_counter(value, state),
            _ => error!("unhandled apu register write at address: 0x{:04X}", address),
        }
    }

    pub(crate) fn step<S: Speaker>(
        &mut self,
        cpu: &mut State,
        cartridge: &mut Box<Cartridge>,
        speaker: &mut S,
    ) {
        self.cycle += 1;
        let cycle1 = self.sample_cycle;
        self.sample_cycle += 1;
        let cycle2 = self.sample_cycle;
        if self.sample_cycle == self.sample_reset_cycle {
            self.sample_cycle = 0;
        }
        self.step_timer(cpu, cartridge);
        self.step_frame_counter(cpu);
        let s1 = cycle1 * u64::from(self.sample_rate) / CLOCK_RATE;
        let s2 = cycle2 * u64::from(self.sample_rate) / CLOCK_RATE;
        if s1 != s2 {
            self.send_sample(speaker);
        }
    }

    pub fn send_sample<S: Speaker>(&mut self, speaker: &mut S) {
        let output = self.output();
        let filtered = self.filter.step(output);
        speaker.push(((filtered * 65535.0) as i32 - 32768) as i16);
    }

    pub fn output(&mut self) -> f32 {
        self.pulse_table[usize::from(self.pulse1.output()) + usize::from(self.pulse2.output())]
            + self.tnd_table[3 * usize::from(self.triangle.output())
                                 + 2 * usize::from(self.noise.output())
                                 + usize::from(self.dmc.output())]
    }

    pub(crate) fn step_frame_counter(&mut self, cpu: &mut State) {
        // https://wiki.nesdev.com/w/index.php/APU_Frame_Counter
        // ここのフレームカウンタをcpuサイクルに合わせて2倍し、6を加えている
        // $4017書き込みによるリセット時には3~4CPUクロックが必要なため。
        if self.frame_period {
            // mode 1 -- 5step
            match self.cycle {
                7462 | 22376 => self.step_envelope(),
                14918 | 37285 => {
                    self.step_envelope();
                    self.step_sweep();
                    self.step_length();
                }
                37288 => self.cycle = 6,
                0...37288 => (),
                _ => unreachable!(),
            }
        } else {
            // mode 0 -- 4step
            match self.cycle {
                7462 | 22376 => self.step_envelope(),
                14917 => {
                    self.step_envelope();
                    self.step_sweep();
                    self.step_length();
                }
                29833 => {
                    self.fire_irq(cpu);
                }
                29834 => {
                    self.step_envelope();
                    self.step_sweep();
                    self.step_length();
                    self.fire_irq(cpu);
                }
                29835 => {
                    self.fire_irq(cpu);
                }
                29836 => self.cycle = 6,
                0...29836 => (),
                _ => unreachable!(),
            }
        }
    }

    fn step_timer(&mut self, cpu: &mut State, cartridge: &mut Box<Cartridge>) {
        if self.cycle & 1 == 0 {
            self.pulse1.step_timer();
            self.pulse2.step_timer();
            self.noise.step_timer();
            self.dmc.step_timer(cpu, cartridge);
        }
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

    fn fire_irq(&mut self, cpu: &mut State) {
        if self.frame_irq {
            cpu.trigger_irq(IrqReason::ApuFrameCounter);
        }
    }

    fn read_status(&mut self, state: &mut State) -> u8 {
        let result = (if self.pulse1.length_value > 0 { 1 } else { 0 })
            | (if self.pulse2.length_value > 0 { 2 } else { 0 })
            | (if self.triangle.length_value > 0 { 4 } else { 0 })
            | (if self.noise.length_value > 0 { 8 } else { 0 })
            | (if self.dmc.length_value > 0 { 0x10 } else { 0 })
            | (if state.get_irq_with_reason(IrqReason::ApuFrameCounter) {
                0x40
            } else {
                0
            })
            | (if state.get_irq_with_reason(IrqReason::ApuDmc) {
                0x80
            } else {
                0
            });
        state.acknowledge_irq(IrqReason::ApuFrameCounter);
        result
    }

    fn write_control(&mut self, value: u8, state: &mut State) {
        self.pulse1.enabled = (value & 1) != 0;
        self.pulse2.enabled = (value & 2) != 0;
        self.triangle.enabled = (value & 4) != 0;
        self.noise.enabled = (value & 8) != 0;
        self.dmc.enabled = (value & 16) != 0;
        if !self.pulse1.enabled {
            self.pulse1.length_value = 0;
        }
        if !self.pulse2.enabled {
            self.pulse2.length_value = 0;
        }
        if !self.triangle.enabled {
            self.triangle.length_value = 0;
        }
        if !self.noise.enabled {
            self.noise.length_value = 0;
        }
        if !self.dmc.enabled {
            self.dmc.length_value = 0;
        } else if self.dmc.length_value == 0 {
            self.dmc.restart(state);
        }
    }

    fn write_frame_counter(&mut self, value: u8, state: &mut State) {
        self.frame_period = ((value >> 7) & 1) != 0;
        self.frame_irq = ((value >> 6) & 1) == 0;
        if self.frame_irq {
            if !state.get_irq_with_reason(IrqReason::ApuFrameCounter) {
                state.enable_irq(IrqReason::ApuFrameCounter);
                state.acknowledge_irq(IrqReason::ApuFrameCounter);
            }
        } else {
            state.disable_irq(IrqReason::ApuFrameCounter);
        }
        self.cycle = 1 + (self.cycle & 1);
        if self.frame_period {
            self.step_envelope();
            self.step_sweep();
            self.step_length();
        }
    }
}
