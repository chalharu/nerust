pub(super) mod absolute;
pub(super) mod absolute_indirect;
pub(super) mod absolute_x;
pub(super) mod absolute_x_rmw;
pub(super) mod absolute_y;
pub(super) mod absolute_y_rmw;
pub(super) mod accumulator;
pub(super) mod immediate;
pub(super) mod implied;
pub(super) mod indexed_indirect;
pub(super) mod indirect_indexed;
pub(super) mod indirect_indexed_rmw;
pub(super) mod relative;
pub(super) mod zero_page;
pub(super) mod zero_page_x;
pub(super) mod zero_page_y;
use super::{Core, CpuStatesEnum, CpuStepStateEnum};

pub(crate) struct AddressingModeLut([CpuStatesEnum; 256]);

impl AddressingModeLut {
    pub(crate) fn new() -> Self {
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

    pub(crate) fn get(&self, code: usize) -> CpuStatesEnum {
        self.0[code]
    }
}

impl Default for AddressingModeLut {
    fn default() -> Self {
        Self::new()
    }
}

fn exit_addressing_mode(core: &mut Core) -> CpuStepStateEnum {
    CpuStepStateEnum::Exit(core.opcode_tables.get(core.internal_stat.get_opcode()))
}
