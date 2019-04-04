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

pub(crate) use self::absolute::Absolute;
pub(crate) use self::absolute_indirect::AbsoluteIndirect;
pub(crate) use self::absolute_x::AbsoluteX;
pub(crate) use self::absolute_x_rmw::AbsoluteXRMW;
pub(crate) use self::absolute_y::AbsoluteY;
pub(crate) use self::absolute_y_rmw::AbsoluteYRMW;
pub(crate) use self::accumulator::Accumulator;
pub(crate) use self::immediate::Immediate;
pub(crate) use self::implied::Implied;
pub(crate) use self::indexed_indirect::IndexedIndirect;
pub(crate) use self::indirect_indexed::IndirectIndexed;
pub(crate) use self::indirect_indexed_rmw::IndirectIndexedRMW;
pub(crate) use self::relative::Relative;
pub(crate) use self::zero_page::ZeroPage;
pub(crate) use self::zero_page_x::ZeroPageX;
pub(crate) use self::zero_page_y::ZeroPageY;
use super::*;

pub(crate) struct AddressingModeLut([CpuStatesEnum; 256]);

impl AddressingModeLut {
    pub fn new() -> Self {
        AddressingModeLut([
            // 0x00
            CpuStatesEnum::Implied,
            CpuStatesEnum::IndexedIndirect,
            CpuStatesEnum::Implied,
            CpuStatesEnum::IndexedIndirect,
            CpuStatesEnum::ZeroPage,
            CpuStatesEnum::ZeroPage,
            CpuStatesEnum::ZeroPage,
            CpuStatesEnum::ZeroPage,
            // 0x08
            CpuStatesEnum::Implied,
            CpuStatesEnum::Immediate,
            CpuStatesEnum::Accumulator,
            CpuStatesEnum::Immediate,
            CpuStatesEnum::Absolute,
            CpuStatesEnum::Absolute,
            CpuStatesEnum::Absolute,
            CpuStatesEnum::Absolute,
            // 0x10
            CpuStatesEnum::Relative,
            CpuStatesEnum::IndirectIndexed,
            CpuStatesEnum::Implied,
            CpuStatesEnum::IndirectIndexedRMW,
            CpuStatesEnum::ZeroPageX,
            CpuStatesEnum::ZeroPageX,
            CpuStatesEnum::ZeroPageX,
            CpuStatesEnum::ZeroPageX,
            // 0x18
            CpuStatesEnum::Implied,
            CpuStatesEnum::AbsoluteY,
            CpuStatesEnum::Implied,
            CpuStatesEnum::AbsoluteYRMW,
            CpuStatesEnum::AbsoluteX,
            CpuStatesEnum::AbsoluteX,
            CpuStatesEnum::AbsoluteXRMW,
            CpuStatesEnum::AbsoluteXRMW,
            // 0x20
            CpuStatesEnum::Absolute,
            CpuStatesEnum::IndexedIndirect,
            CpuStatesEnum::Implied,
            CpuStatesEnum::IndexedIndirect,
            CpuStatesEnum::ZeroPage,
            CpuStatesEnum::ZeroPage,
            CpuStatesEnum::ZeroPage,
            CpuStatesEnum::ZeroPage,
            // 0x28
            CpuStatesEnum::Implied,
            CpuStatesEnum::Immediate,
            CpuStatesEnum::Accumulator,
            CpuStatesEnum::Immediate,
            CpuStatesEnum::Absolute,
            CpuStatesEnum::Absolute,
            CpuStatesEnum::Absolute,
            CpuStatesEnum::Absolute,
            // 0x30
            CpuStatesEnum::Relative,
            CpuStatesEnum::IndirectIndexed,
            CpuStatesEnum::Implied,
            CpuStatesEnum::IndirectIndexedRMW,
            CpuStatesEnum::ZeroPageX,
            CpuStatesEnum::ZeroPageX,
            CpuStatesEnum::ZeroPageX,
            CpuStatesEnum::ZeroPageX,
            // 0x38
            CpuStatesEnum::Implied,
            CpuStatesEnum::AbsoluteY,
            CpuStatesEnum::Implied,
            CpuStatesEnum::AbsoluteYRMW,
            CpuStatesEnum::AbsoluteX,
            CpuStatesEnum::AbsoluteX,
            CpuStatesEnum::AbsoluteXRMW,
            CpuStatesEnum::AbsoluteXRMW,
            // 0x40
            CpuStatesEnum::Implied,
            CpuStatesEnum::IndexedIndirect,
            CpuStatesEnum::Implied,
            CpuStatesEnum::IndexedIndirect,
            CpuStatesEnum::ZeroPage,
            CpuStatesEnum::ZeroPage,
            CpuStatesEnum::ZeroPage,
            CpuStatesEnum::ZeroPage,
            // 0x48
            CpuStatesEnum::Implied,
            CpuStatesEnum::Immediate,
            CpuStatesEnum::Accumulator,
            CpuStatesEnum::Immediate,
            CpuStatesEnum::Absolute,
            CpuStatesEnum::Absolute,
            CpuStatesEnum::Absolute,
            CpuStatesEnum::Absolute,
            // 0x50
            CpuStatesEnum::Relative,
            CpuStatesEnum::IndirectIndexed,
            CpuStatesEnum::Implied,
            CpuStatesEnum::IndirectIndexedRMW,
            CpuStatesEnum::ZeroPageX,
            CpuStatesEnum::ZeroPageX,
            CpuStatesEnum::ZeroPageX,
            CpuStatesEnum::ZeroPageX,
            // 0x58
            CpuStatesEnum::Implied,
            CpuStatesEnum::AbsoluteY,
            CpuStatesEnum::Implied,
            CpuStatesEnum::AbsoluteYRMW,
            CpuStatesEnum::AbsoluteX,
            CpuStatesEnum::AbsoluteX,
            CpuStatesEnum::AbsoluteXRMW,
            CpuStatesEnum::AbsoluteXRMW,
            // 0x60
            CpuStatesEnum::Implied,
            CpuStatesEnum::IndexedIndirect,
            CpuStatesEnum::Implied,
            CpuStatesEnum::IndexedIndirect,
            CpuStatesEnum::ZeroPage,
            CpuStatesEnum::ZeroPage,
            CpuStatesEnum::ZeroPage,
            CpuStatesEnum::ZeroPage,
            // 0x68
            CpuStatesEnum::Implied,
            CpuStatesEnum::Immediate,
            CpuStatesEnum::Accumulator,
            CpuStatesEnum::Immediate,
            CpuStatesEnum::AbsoluteIndirect,
            CpuStatesEnum::Absolute,
            CpuStatesEnum::Absolute,
            CpuStatesEnum::Absolute,
            // 0x70
            CpuStatesEnum::Relative,
            CpuStatesEnum::IndirectIndexed,
            CpuStatesEnum::Implied,
            CpuStatesEnum::IndirectIndexedRMW,
            CpuStatesEnum::ZeroPageX,
            CpuStatesEnum::ZeroPageX,
            CpuStatesEnum::ZeroPageX,
            CpuStatesEnum::ZeroPageX,
            // 0x78
            CpuStatesEnum::Implied,
            CpuStatesEnum::AbsoluteY,
            CpuStatesEnum::Implied,
            CpuStatesEnum::AbsoluteYRMW,
            CpuStatesEnum::AbsoluteX,
            CpuStatesEnum::AbsoluteX,
            CpuStatesEnum::AbsoluteXRMW,
            CpuStatesEnum::AbsoluteXRMW,
            // 0x80
            CpuStatesEnum::Immediate,
            CpuStatesEnum::IndexedIndirect,
            CpuStatesEnum::Immediate,
            CpuStatesEnum::IndexedIndirect,
            CpuStatesEnum::ZeroPage,
            CpuStatesEnum::ZeroPage,
            CpuStatesEnum::ZeroPage,
            CpuStatesEnum::ZeroPage,
            // 0x88
            CpuStatesEnum::Implied,
            CpuStatesEnum::Immediate,
            CpuStatesEnum::Implied,
            CpuStatesEnum::Immediate,
            CpuStatesEnum::Absolute,
            CpuStatesEnum::Absolute,
            CpuStatesEnum::Absolute,
            CpuStatesEnum::Absolute,
            // 0x90
            CpuStatesEnum::Relative,
            CpuStatesEnum::IndirectIndexedRMW,
            CpuStatesEnum::Implied,
            CpuStatesEnum::IndirectIndexedRMW,
            CpuStatesEnum::ZeroPageX,
            CpuStatesEnum::ZeroPageX,
            CpuStatesEnum::ZeroPageY,
            CpuStatesEnum::ZeroPageY,
            // 0x98
            CpuStatesEnum::Implied,
            CpuStatesEnum::AbsoluteYRMW,
            CpuStatesEnum::Implied,
            CpuStatesEnum::AbsoluteYRMW,
            CpuStatesEnum::AbsoluteXRMW,
            CpuStatesEnum::AbsoluteXRMW,
            CpuStatesEnum::AbsoluteYRMW,
            CpuStatesEnum::AbsoluteYRMW,
            // 0xA0
            CpuStatesEnum::Immediate,
            CpuStatesEnum::IndexedIndirect,
            CpuStatesEnum::Immediate,
            CpuStatesEnum::IndexedIndirect,
            CpuStatesEnum::ZeroPage,
            CpuStatesEnum::ZeroPage,
            CpuStatesEnum::ZeroPage,
            CpuStatesEnum::ZeroPage,
            // 0xA8
            CpuStatesEnum::Implied,
            CpuStatesEnum::Immediate,
            CpuStatesEnum::Implied,
            CpuStatesEnum::Immediate,
            CpuStatesEnum::Absolute,
            CpuStatesEnum::Absolute,
            CpuStatesEnum::Absolute,
            CpuStatesEnum::Absolute,
            // 0xB0
            CpuStatesEnum::Relative,
            CpuStatesEnum::IndirectIndexed,
            CpuStatesEnum::Implied,
            CpuStatesEnum::IndirectIndexed,
            CpuStatesEnum::ZeroPageX,
            CpuStatesEnum::ZeroPageX,
            CpuStatesEnum::ZeroPageY,
            CpuStatesEnum::ZeroPageY,
            // 0xB8
            CpuStatesEnum::Implied,
            CpuStatesEnum::AbsoluteY,
            CpuStatesEnum::Implied,
            CpuStatesEnum::AbsoluteY,
            CpuStatesEnum::AbsoluteX,
            CpuStatesEnum::AbsoluteX,
            CpuStatesEnum::AbsoluteY,
            CpuStatesEnum::AbsoluteY,
            // 0xC0
            CpuStatesEnum::Immediate,
            CpuStatesEnum::IndexedIndirect,
            CpuStatesEnum::Immediate,
            CpuStatesEnum::IndexedIndirect,
            CpuStatesEnum::ZeroPage,
            CpuStatesEnum::ZeroPage,
            CpuStatesEnum::ZeroPage,
            CpuStatesEnum::ZeroPage,
            // 0xC8
            CpuStatesEnum::Implied,
            CpuStatesEnum::Immediate,
            CpuStatesEnum::Implied,
            CpuStatesEnum::Immediate,
            CpuStatesEnum::Absolute,
            CpuStatesEnum::Absolute,
            CpuStatesEnum::Absolute,
            CpuStatesEnum::Absolute,
            // 0xD0
            CpuStatesEnum::Relative,
            CpuStatesEnum::IndirectIndexed,
            CpuStatesEnum::Implied,
            CpuStatesEnum::IndirectIndexedRMW,
            CpuStatesEnum::ZeroPageX,
            CpuStatesEnum::ZeroPageX,
            CpuStatesEnum::ZeroPageX,
            CpuStatesEnum::ZeroPageX,
            // 0xD8
            CpuStatesEnum::Implied,
            CpuStatesEnum::AbsoluteY,
            CpuStatesEnum::Implied,
            CpuStatesEnum::AbsoluteYRMW,
            CpuStatesEnum::AbsoluteX,
            CpuStatesEnum::AbsoluteX,
            CpuStatesEnum::AbsoluteXRMW,
            CpuStatesEnum::AbsoluteXRMW,
            // 0xE0
            CpuStatesEnum::Immediate,
            CpuStatesEnum::IndexedIndirect,
            CpuStatesEnum::Immediate,
            CpuStatesEnum::IndexedIndirect,
            CpuStatesEnum::ZeroPage,
            CpuStatesEnum::ZeroPage,
            CpuStatesEnum::ZeroPage,
            CpuStatesEnum::ZeroPage,
            // 0xE8
            CpuStatesEnum::Implied,
            CpuStatesEnum::Immediate,
            CpuStatesEnum::Implied,
            CpuStatesEnum::Immediate,
            CpuStatesEnum::Absolute,
            CpuStatesEnum::Absolute,
            CpuStatesEnum::Absolute,
            CpuStatesEnum::Absolute,
            // 0xF0
            CpuStatesEnum::Relative,
            CpuStatesEnum::IndirectIndexed,
            CpuStatesEnum::Implied,
            CpuStatesEnum::IndirectIndexedRMW,
            CpuStatesEnum::ZeroPageX,
            CpuStatesEnum::ZeroPageX,
            CpuStatesEnum::ZeroPageX,
            CpuStatesEnum::ZeroPageX,
            // 0xF8
            CpuStatesEnum::Implied,
            CpuStatesEnum::AbsoluteY,
            CpuStatesEnum::Implied,
            CpuStatesEnum::AbsoluteYRMW,
            CpuStatesEnum::AbsoluteX,
            CpuStatesEnum::AbsoluteX,
            CpuStatesEnum::AbsoluteXRMW,
            CpuStatesEnum::AbsoluteXRMW,
        ])
    }

    pub fn get(&self, code: usize) -> CpuStatesEnum {
        self.0[code]
    }
}

fn exit_addressing_mode(core: &mut Core) -> CpuStepStateEnum {
    CpuStepStateEnum::Exit(core.opcode_tables.get(core.register.get_opcode()))
}
