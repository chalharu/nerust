// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use crate::nes::cpu::interrupt::*;
use crate::nes::Cartridge;
use crate::nes::Cpu;

// NTSC
// https://wiki.nesdev.com/w/index.php/APU_DMC
// 2で1APUサイクル
const DMC_TABLE: [u8; 16] = [
    214, 190, 170, 160, 143, 127, 113, 107, 95, 80, 71, 64, 53, 42, 36, 27,
];

pub(crate) struct DMC {
    pub enabled: bool,
    value: u8,
    sample_address: u16,
    sample_length: u16,
    current_address: u16,
    pub length_value: u16,
    shift_register: u8,
    bit_count: u8,
    tick_period: u8,
    tick_value: u8,
    is_loop: bool,
    irq: bool,
    read_buffer: u8,
}

impl DMC {
    pub fn new() -> Self {
        Self {
            enabled: false,
            value: 0,
            sample_address: 0,
            sample_length: 0,
            current_address: 0,
            length_value: 0,
            shift_register: 0,
            bit_count: 0,
            tick_period: 0,
            tick_value: 0,
            is_loop: false,
            irq: false,
            read_buffer: 0,
        }
    }

    pub fn write_control(&mut self, value: u8, interrupt: &mut Interrupt) {
        self.irq = (value & 0x80) != 0;
        self.is_loop = (value & 0x40) != 0;
        self.tick_period = DMC_TABLE[usize::from(value & 0x0f)];
        if self.irq {
            interrupt.clear_irq(IrqSource::DMC);
        }
    }

    pub fn write_value(&mut self, value: u8) {
        self.value = value & 0x7F;
    }

    pub fn write_address(&mut self, value: u8) {
        self.sample_address = 0xC000 | (u16::from(value) << 6);
    }

    pub fn write_length(&mut self, value: u8) {
        if self.enabled {
            self.sample_length = 1 | (u16::from(value) << 4);
        }
    }

    pub fn restart(&mut self) {
        self.current_address = self.sample_address;
        self.length_value = self.sample_length;
    }

    pub fn step_timer(&mut self, cpu: &mut Cpu, cartridge: &mut Box<Cartridge>) {
        if self.enabled {
            self.step_reader(cpu, cartridge);
            if self.tick_value == 0 {
                self.tick_value = self.tick_period;
                self.step_shifter();
            } else {
                self.tick_value -= 1;
            }
        }
    }

    pub fn fill_address(&mut self) -> usize {
        self.current_address as usize
    }

    pub fn fill(&mut self, value: u8, interrupt: &mut Interrupt) {
        if self.length_value > 0 {
            self.read_buffer = value;
            self.current_address = self.current_address.wrapping_add(1);
            if self.current_address == 0 {
                self.current_address = 0x8000;
            }
            self.length_value -= 1;
            if self.length_value == 0 {
                if self.is_loop {
                    self.restart();
                } else if self.irq {
                    interrupt.set_irq(IrqSource::DMC);
                }
            }
        }
    }

    pub fn step_reader(&mut self, _cpu: &mut Cpu, _cartridge: &mut Box<Cartridge>) {
        if self.length_value > 0 && self.bit_count == 0 {
            self.shift_register = self.read_buffer;
            self.bit_count = 8;
        }
    }

    pub fn step_shifter(&mut self) {
        if self.bit_count != 0 {
            if ((self.shift_register & 1) != 1) && (self.value <= 125) {
                self.value += 2;
            } else if self.value >= 2 {
                self.value -= 2;
            }
            self.shift_register >>= 1;
            self.bit_count -= 1;
        }
    }

    pub fn output(&self) -> u8 {
        self.value
    }
}
