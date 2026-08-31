use crate::cpu::registers::CpuRegisters;
use crate::memory::GbaMemoryBus;
pub fn handle_pc_relative(_regs: &mut CpuRegisters, _bus: &mut GbaMemoryBus, _instr: u16) -> u32 {
    1
}
pub fn handle_reg_offset(_regs: &mut CpuRegisters, _bus: &mut GbaMemoryBus, _instr: u16) -> u32 {
    1
}
pub fn handle_imm_offset(_regs: &mut CpuRegisters, _bus: &mut GbaMemoryBus, _instr: u16) -> u32 {
    1
}
pub fn handle_halfword(_regs: &mut CpuRegisters, _bus: &mut GbaMemoryBus, _instr: u16) -> u32 {
    1
}
pub fn handle_sp_relative(_regs: &mut CpuRegisters, _bus: &mut GbaMemoryBus, _instr: u16) -> u32 {
    1
}
pub fn handle_multiple(_regs: &mut CpuRegisters, _bus: &mut GbaMemoryBus, _instr: u16) -> u32 {
    1
}
