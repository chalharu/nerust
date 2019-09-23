// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use crate::cpu::interrupt::{Interrupt, IrqSource};

#[derive(serde_derive::Serialize, serde_derive::Deserialize, Debug, Eq, PartialEq)]
pub(crate) enum FrameType {
    None,
    Quarter,
    Half,
}

#[derive(serde_derive::Serialize, serde_derive::Deserialize, Debug, Copy, Clone)]
pub(crate) struct FrameCounter {
    period: bool,
    irq: bool,
    write_counter: usize,
    block: usize,
    new_value: u8,
    clock_cycle: u64,
    cycle: u16,
}

impl FrameCounter {
    pub(crate) fn new() -> Self {
        Self {
            period: false,
            irq: true,
            write_counter: 3,
            block: 0,
            new_value: 0,
            clock_cycle: 0,
            cycle: 0,
        }
    }

    pub(crate) fn reset(&mut self) {
        self.soft_reset();
        self.period = false;
        self.new_value = 0;
    }

    pub(crate) fn soft_reset(&mut self) {
        self.irq = true;
        self.write_counter = 3;
        self.new_value = if self.period { 0x80 } else { 0 };
        self.clock_cycle = 0;
        self.cycle = 0;
        self.block = 2;
    }

    fn fire_irq(&self, interrupt: &mut Interrupt) {
        if self.irq {
            interrupt.set_irq(IrqSource::FRAME_COUNTER);
        }
    }

    pub(crate) fn step_frame_counter(&mut self, interrupt: &mut Interrupt) -> FrameType {
        self.clock_cycle = self.clock_cycle.wrapping_add(1);
        self.cycle += 1;

        // https://wiki.nesdev.com/w/index.php/APU_Frame_Counter
        let mut result = if self.period {
            // mode 1 -- 5step
            match self.cycle {
                7457 | 22371 => FrameType::Quarter,
                14913 | 37281 => FrameType::Half,
                37282 => {
                    self.cycle = 0;
                    FrameType::None
                }
                0..=7456 | 7458..=14912 | 14914..=22370 | 22372..=37280 => FrameType::None,
                _ => unreachable!(),
            }
        } else {
            // mode 0 -- 4step
            match self.cycle {
                7457 | 22371 => FrameType::Quarter,
                14913 => FrameType::Half,
                29828 => {
                    self.fire_irq(interrupt);
                    FrameType::None
                }
                29829 => {
                    self.fire_irq(interrupt);
                    FrameType::Half
                }
                29830 => {
                    self.fire_irq(interrupt);
                    self.cycle = 0;
                    FrameType::None
                }
                0..=7456 | 7458..=14912 | 14914..=22370 | 22372..=29827 => FrameType::None,
                _ => unreachable!(),
            }
        };

        if result != FrameType::None {
            if self.block == 0 {
                self.block = 2;
            } else {
                result = FrameType::None
            }
        }
        if self.write_counter > 0 {
            self.write_counter -= 1;
            if self.write_counter == 0 {
                self.period = (self.new_value & 0x80) != 0;
                self.cycle = 0;
                if self.period && self.block == 0 {
                    result = FrameType::Half;
                    self.block = 2;
                }
            }
        }

        if self.block > 0 {
            self.block -= 1;
        }

        result
    }

    pub(crate) fn write_frame_counter(&mut self, value: u8, interrupt: &mut Interrupt) {
        self.irq = ((value >> 6) & 1) == 0;
        self.new_value = value;
        if (self.clock_cycle & 1) != 0 {
            self.write_counter = 3;
        } else {
            self.write_counter = 4;
        }
        if !self.irq {
            interrupt.clear_irq(IrqSource::FRAME_COUNTER);
        }
    }
}
