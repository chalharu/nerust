//! Shared register access helpers used by multiple opcode modules.

use crate::cpu_core::Lr35902Cpu;

/// Named register indices for use as const generic parameters.
pub(crate) mod reg {
    // r8 registers (Pan Docs encoding: bits 0-2 or bits 3-5)
    pub(crate) const B: u8 = 0;
    pub(crate) const C: u8 = 1;
    pub(crate) const D: u8 = 2;
    pub(crate) const E: u8 = 3;
    pub(crate) const H: u8 = 4;
    pub(crate) const L: u8 = 5;
    pub(crate) const HL: u8 = 6;
    pub(crate) const A: u8 = 7;

    // r16 register pairs (bits 4-5)
    pub(crate) const BC: u8 = 0;
    pub(crate) const DE: u8 = 1;
    pub(crate) const R16_HL: u8 = 2;
    pub(crate) const SP: u8 = 3;
    pub(crate) const AF: u8 = 3; // same as SP, used by PUSH/POP
}

pub(crate) fn read_r8(core: &Lr35902Cpu, idx: u8) -> u8 {
    match idx {
        0 => core.registers.b,
        1 => core.registers.c,
        2 => core.registers.d,
        3 => core.registers.e,
        4 => core.registers.h,
        5 => core.registers.l,
        7 => core.registers.a,
        _ => 0,
    }
}

pub(crate) fn write_r8(core: &mut Lr35902Cpu, idx: u8, v: u8) {
    match idx {
        0 => core.registers.b = v,
        1 => core.registers.c = v,
        2 => core.registers.d = v,
        3 => core.registers.e = v,
        4 => core.registers.h = v,
        5 => core.registers.l = v,
        7 => core.registers.a = v,
        _ => {}
    }
}

pub(crate) fn r8_from_opcode(opcode: u8) -> u8 {
    opcode & 0x07
}

pub(crate) fn r16_from_opcode(opcode: u8) -> u8 {
    (opcode >> 4) & 0x03
}

pub(crate) fn read_r16(core: &Lr35902Cpu, idx: u8) -> u16 {
    match idx {
        0 => core.registers.bc(),
        1 => core.registers.de(),
        2 => core.registers.hl(),
        _ => core.registers.sp,
    }
}

pub(crate) fn write_r16(core: &mut Lr35902Cpu, idx: u8, v: u16) {
    match idx {
        0 => core.registers.set_bc(v),
        1 => core.registers.set_de(v),
        2 => core.registers.set_hl(v),
        _ => core.registers.sp = v,
    }
}
