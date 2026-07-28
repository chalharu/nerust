//! Shared register access helpers used by multiple opcode modules.

use crate::cpu_core::Lr35902Cpu;

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
