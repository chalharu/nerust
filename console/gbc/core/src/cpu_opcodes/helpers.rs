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
        reg::B => core.registers().b(),
        reg::C => core.registers().c(),
        reg::D => core.registers().d(),
        reg::E => core.registers().e(),
        reg::H => core.registers().h(),
        reg::L => core.registers().l(),
        reg::A => core.registers().a(),
        _ => 0,
    }
}

pub(crate) fn write_r8(core: &mut Lr35902Cpu, idx: u8, v: u8) {
    match idx {
        reg::B => core.registers_mut().set_b(v),
        reg::C => core.registers_mut().set_c(v),
        reg::D => core.registers_mut().set_d(v),
        reg::E => core.registers_mut().set_e(v),
        reg::H => core.registers_mut().set_h(v),
        reg::L => core.registers_mut().set_l(v),
        reg::A => core.registers_mut().set_a(v),
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
        reg::BC => core.registers().bc(),
        reg::DE => core.registers().de(),
        reg::R16_HL => core.registers().hl(),
        _ => core.registers().sp(),
    }
}

pub(crate) fn write_r16(core: &mut Lr35902Cpu, idx: u8, v: u16) {
    match idx {
        reg::BC => core.registers_mut().set_bc(v),
        reg::DE => core.registers_mut().set_de(v),
        reg::R16_HL => core.registers_mut().set_hl(v),
        _ => core.registers_mut().set_sp(v),
    }
}
