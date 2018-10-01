// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

mod absolute;
mod absolute_indirect;
mod absolute_x;
mod absolute_x_rmw;
mod absolute_y;
mod absolute_y_rmw;
mod accumulator;
mod immediate;
mod implied;
mod indexed_indirect;
mod indirect_indexed;
mod indirect_indexed_rmw;
mod relative;
mod zero_page;
mod zero_page_x;
mod zero_page_y;

use self::absolute::Absolute;
use self::absolute_indirect::AbsoluteIndirect;
use self::absolute_x::AbsoluteX;
use self::absolute_x_rmw::AbsoluteXRMW;
use self::absolute_y::AbsoluteY;
use self::absolute_y_rmw::AbsoluteYRMW;
use self::accumulator::Accumulator;
use self::immediate::Immediate;
use self::implied::Implied;
use self::indexed_indirect::IndexedIndirect;
use self::indirect_indexed::IndirectIndexed;
use self::indirect_indexed_rmw::IndirectIndexedRMW;
use self::relative::Relative;
use self::zero_page::ZeroPage;
use self::zero_page_x::ZeroPageX;
use self::zero_page_y::ZeroPageY;
use super::*;

pub(crate) trait AddressingMode {
    fn next_func(
        &self,
        code: usize,
        register: &mut Register,
        opcodes: &mut Opcodes,
        interrupt: &mut Interrupt,
    ) -> Box<dyn CpuStepState>;
    fn name(&self) -> &'static str;
    // fn opcode_length(&self) -> u16;
}

pub(crate) struct AddressingModeLut([Box<AddressingMode>; 256]);

impl AddressingModeLut {
    pub fn new() -> Self {
        AddressingModeLut([
            // 0x00
            Box::new(Implied),
            Box::new(IndexedIndirect),
            Box::new(Implied),
            Box::new(IndexedIndirect),
            Box::new(ZeroPage),
            Box::new(ZeroPage),
            Box::new(ZeroPage),
            Box::new(ZeroPage),
            // 0x08
            Box::new(Implied),
            Box::new(Immediate),
            Box::new(Accumulator),
            Box::new(Immediate),
            Box::new(Absolute),
            Box::new(Absolute),
            Box::new(Absolute),
            Box::new(Absolute),
            // 0x10
            Box::new(Relative),
            Box::new(IndirectIndexed),
            Box::new(Implied),
            Box::new(IndirectIndexedRMW),
            Box::new(ZeroPageX),
            Box::new(ZeroPageX),
            Box::new(ZeroPageX),
            Box::new(ZeroPageX),
            // 0x18
            Box::new(Implied),
            Box::new(AbsoluteY),
            Box::new(Implied),
            Box::new(AbsoluteYRMW),
            Box::new(AbsoluteX),
            Box::new(AbsoluteX),
            Box::new(AbsoluteXRMW),
            Box::new(AbsoluteXRMW),
            // 0x20
            Box::new(Absolute),
            Box::new(IndexedIndirect),
            Box::new(Implied),
            Box::new(IndexedIndirect),
            Box::new(ZeroPage),
            Box::new(ZeroPage),
            Box::new(ZeroPage),
            Box::new(ZeroPage),
            // 0x28
            Box::new(Implied),
            Box::new(Immediate),
            Box::new(Accumulator),
            Box::new(Immediate),
            Box::new(Absolute),
            Box::new(Absolute),
            Box::new(Absolute),
            Box::new(Absolute),
            // 0x30
            Box::new(Relative),
            Box::new(IndirectIndexed),
            Box::new(Implied),
            Box::new(IndirectIndexedRMW),
            Box::new(ZeroPageX),
            Box::new(ZeroPageX),
            Box::new(ZeroPageX),
            Box::new(ZeroPageX),
            // 0x38
            Box::new(Implied),
            Box::new(AbsoluteY),
            Box::new(Implied),
            Box::new(AbsoluteYRMW),
            Box::new(AbsoluteX),
            Box::new(AbsoluteX),
            Box::new(AbsoluteXRMW),
            Box::new(AbsoluteXRMW),
            // 0x40
            Box::new(Implied),
            Box::new(IndexedIndirect),
            Box::new(Implied),
            Box::new(IndexedIndirect),
            Box::new(ZeroPage),
            Box::new(ZeroPage),
            Box::new(ZeroPage),
            Box::new(ZeroPage),
            // 0x48
            Box::new(Implied),
            Box::new(Immediate),
            Box::new(Accumulator),
            Box::new(Immediate),
            Box::new(Absolute),
            Box::new(Absolute),
            Box::new(Absolute),
            Box::new(Absolute),
            // 0x50
            Box::new(Relative),
            Box::new(IndirectIndexed),
            Box::new(Implied),
            Box::new(IndirectIndexedRMW),
            Box::new(ZeroPageX),
            Box::new(ZeroPageX),
            Box::new(ZeroPageX),
            Box::new(ZeroPageX),
            // 0x58
            Box::new(Implied),
            Box::new(AbsoluteY),
            Box::new(Implied),
            Box::new(AbsoluteYRMW),
            Box::new(AbsoluteX),
            Box::new(AbsoluteX),
            Box::new(AbsoluteXRMW),
            Box::new(AbsoluteXRMW),
            // 0x60
            Box::new(Implied),
            Box::new(IndexedIndirect),
            Box::new(Implied),
            Box::new(IndexedIndirect),
            Box::new(ZeroPage),
            Box::new(ZeroPage),
            Box::new(ZeroPage),
            Box::new(ZeroPage),
            // 0x68
            Box::new(Implied),
            Box::new(Immediate),
            Box::new(Accumulator),
            Box::new(Immediate),
            Box::new(AbsoluteIndirect),
            Box::new(Absolute),
            Box::new(Absolute),
            Box::new(Absolute),
            // 0x70
            Box::new(Relative),
            Box::new(IndirectIndexed),
            Box::new(Implied),
            Box::new(IndirectIndexedRMW),
            Box::new(ZeroPageX),
            Box::new(ZeroPageX),
            Box::new(ZeroPageX),
            Box::new(ZeroPageX),
            // 0x78
            Box::new(Implied),
            Box::new(AbsoluteY),
            Box::new(Implied),
            Box::new(AbsoluteYRMW),
            Box::new(AbsoluteX),
            Box::new(AbsoluteX),
            Box::new(AbsoluteXRMW),
            Box::new(AbsoluteXRMW),
            // 0x80
            Box::new(Immediate),
            Box::new(IndexedIndirect),
            Box::new(Immediate),
            Box::new(IndexedIndirect),
            Box::new(ZeroPage),
            Box::new(ZeroPage),
            Box::new(ZeroPage),
            Box::new(ZeroPage),
            // 0x88
            Box::new(Implied),
            Box::new(Immediate),
            Box::new(Implied),
            Box::new(Immediate),
            Box::new(Absolute),
            Box::new(Absolute),
            Box::new(Absolute),
            Box::new(Absolute),
            // 0x90
            Box::new(Relative),
            Box::new(IndirectIndexedRMW),
            Box::new(Implied),
            Box::new(IndirectIndexedRMW),
            Box::new(ZeroPageX),
            Box::new(ZeroPageX),
            Box::new(ZeroPageY),
            Box::new(ZeroPageY),
            // 0x98
            Box::new(Implied),
            Box::new(AbsoluteYRMW),
            Box::new(Implied),
            Box::new(AbsoluteYRMW),
            Box::new(AbsoluteXRMW),
            Box::new(AbsoluteXRMW),
            Box::new(AbsoluteYRMW),
            Box::new(AbsoluteYRMW),
            // 0xA0
            Box::new(Immediate),
            Box::new(IndexedIndirect),
            Box::new(Immediate),
            Box::new(IndexedIndirect),
            Box::new(ZeroPage),
            Box::new(ZeroPage),
            Box::new(ZeroPage),
            Box::new(ZeroPage),
            // 0xA8
            Box::new(Implied),
            Box::new(Immediate),
            Box::new(Implied),
            Box::new(Immediate),
            Box::new(Absolute),
            Box::new(Absolute),
            Box::new(Absolute),
            Box::new(Absolute),
            // 0xB0
            Box::new(Relative),
            Box::new(IndirectIndexed),
            Box::new(Implied),
            Box::new(IndirectIndexed),
            Box::new(ZeroPageX),
            Box::new(ZeroPageX),
            Box::new(ZeroPageY),
            Box::new(ZeroPageY),
            // 0xB8
            Box::new(Implied),
            Box::new(AbsoluteY),
            Box::new(Implied),
            Box::new(AbsoluteY),
            Box::new(AbsoluteX),
            Box::new(AbsoluteX),
            Box::new(AbsoluteY),
            Box::new(AbsoluteY),
            // 0xC0
            Box::new(Immediate),
            Box::new(IndexedIndirect),
            Box::new(Immediate),
            Box::new(IndexedIndirect),
            Box::new(ZeroPage),
            Box::new(ZeroPage),
            Box::new(ZeroPage),
            Box::new(ZeroPage),
            // 0xC8
            Box::new(Implied),
            Box::new(Immediate),
            Box::new(Implied),
            Box::new(Immediate),
            Box::new(Absolute),
            Box::new(Absolute),
            Box::new(Absolute),
            Box::new(Absolute),
            // 0xD0
            Box::new(Relative),
            Box::new(IndirectIndexed),
            Box::new(Implied),
            Box::new(IndirectIndexedRMW),
            Box::new(ZeroPageX),
            Box::new(ZeroPageX),
            Box::new(ZeroPageX),
            Box::new(ZeroPageX),
            // 0xD8
            Box::new(Implied),
            Box::new(AbsoluteY),
            Box::new(Implied),
            Box::new(AbsoluteYRMW),
            Box::new(AbsoluteX),
            Box::new(AbsoluteX),
            Box::new(AbsoluteXRMW),
            Box::new(AbsoluteXRMW),
            // 0xE0
            Box::new(Immediate),
            Box::new(IndexedIndirect),
            Box::new(Immediate),
            Box::new(IndexedIndirect),
            Box::new(ZeroPage),
            Box::new(ZeroPage),
            Box::new(ZeroPage),
            Box::new(ZeroPage),
            // 0xE8
            Box::new(Implied),
            Box::new(Immediate),
            Box::new(Implied),
            Box::new(Immediate),
            Box::new(Absolute),
            Box::new(Absolute),
            Box::new(Absolute),
            Box::new(Absolute),
            // 0xF0
            Box::new(Relative),
            Box::new(IndirectIndexed),
            Box::new(Implied),
            Box::new(IndirectIndexedRMW),
            Box::new(ZeroPageX),
            Box::new(ZeroPageX),
            Box::new(ZeroPageX),
            Box::new(ZeroPageX),
            // 0xF8
            Box::new(Implied),
            Box::new(AbsoluteY),
            Box::new(Implied),
            Box::new(AbsoluteYRMW),
            Box::new(AbsoluteX),
            Box::new(AbsoluteX),
            Box::new(AbsoluteXRMW),
            Box::new(AbsoluteXRMW),
        ])
    }

    pub fn get(&self, code: usize) -> &Box<dyn AddressingMode> {
        &self.0[code]
    }
}
