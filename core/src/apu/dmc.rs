// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use super::timer::*;
use crate::cpu::interrupt::*;
use crate::Cartridge;
use std::mem;

// NTSC
// https://wiki.nesdev.com/w/index.php/APU_DMC
// 2で1APUサイクル
const DMC_TABLE: [u8; 16] = [
    214, 190, 170, 160, 143, 127, 113, 107, 95, 80, 71, 64, 53, 42, 36, 27,
];

#[derive(Serialize, Deserialize, Debug, Copy, Clone)]
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
    prev_reg_value: u8,
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
    pub fn new() -> Self {
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
            prev_reg_value: 0,
            timer: TimerDao::new(),
        }
    }

    pub fn reset(&mut self) {
        self.timer.reset();
        let period = (u16::from(DMC_TABLE[0]) << 1) - 1;
        self.timer.set_period(period);
        self.timer.set_value(period);
        self.is_loop = false;
        self.irq = false;
    }

    pub fn write_control(&mut self, value: u8, interrupt: &mut Interrupt) {
        self.irq = (value & 0x80) != 0;
        self.is_loop = (value & 0x40) != 0;
        self.timer
            .set_period((u16::from(DMC_TABLE[usize::from(value & 0x0f)]) << 1) - 1);
        if !self.irq {
            interrupt.clear_irq(IrqSource::DMC);
        }
    }

    pub fn write_value(&mut self, value: u8) {
        let prev_value = mem::replace(&mut self.value, value & 0x7F);
        self.prev_reg_value = self.value;

        let output_diff = if self.value > prev_value {
            self.value - prev_value
        } else {
            prev_value - self.value
        };
        if output_diff > 50 {
            if self.value > prev_value {
                self.value -= output_diff >> 1;
            } else {
                self.value += output_diff >> 1;
            }
        }
    }

    pub fn write_address(&mut self, value: u8) {
        self.sample_address = 0xC000 | (u16::from(value) << 6);
    }

    pub fn write_length(&mut self, value: u8) {
        self.sample_length = 1 | (u16::from(value) << 4);
    }

    pub fn set_enabled(&mut self, enabled: bool, interrupt: &mut Interrupt) {
        if !enabled {
            self.length_value = 0;
        } else if self.length_value == 0 {
            self.restart();
            if self.need_buffer && self.length_value > 0 {
                interrupt.dmc_start = true;
            }
        }
    }

    pub fn restart(&mut self) {
        self.current_address = self.sample_address;
        self.length_value = self.sample_length;
    }

    pub fn fill_address(&self) -> Option<usize> {
        if self.get_status() {
            Some(self.current_address as usize)
        } else {
            None
        }
    }

    pub fn get_status(&self) -> bool {
        self.length_value > 0
    }

    pub fn fill(&mut self, value: u8, interrupt: &mut Interrupt) {
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

    pub fn step_timer(&mut self, interrupt: &mut Interrupt, cartridge: &mut dyn Cartridge) {
        if self.timer.step_timer() {
            if self.enabled {
                self.step_shifter();
            }
            if self.bit_count > 0 {
                self.bit_count -= 1;
            }

            self.step_reader(interrupt, cartridge);
        }
    }

    pub fn step_reader(&mut self, interrupt: &mut Interrupt, _cartridge: &mut dyn Cartridge) {
        if self.bit_count == 0 {
            self.bit_count = 8;
            if self.need_buffer {
                self.enabled = false;
            } else {
                self.enabled = true;
                self.shift_register = self.read_buffer;
                self.need_buffer = true;
                interrupt.dmc_start = true;
            }
        }
    }

    pub fn step_shifter(&mut self) {
        if (self.shift_register & 1) != 0 {
            if self.value <= 125 {
                self.value += 2;
            }
        } else {
            self.value -= 2;
        }

        self.shift_register >>= 1;
    }

    pub fn output(&self) -> u8 {
        self.value
    }
}
