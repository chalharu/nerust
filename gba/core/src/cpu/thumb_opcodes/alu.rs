use crate::cpu::registers::CpuRegisters;
use crate::memory::GbaMemoryBus;
pub fn handle(_regs: &mut CpuRegisters, _instr: u16) -> u32 {
    1
}
pub fn handle_imm(_regs: &mut CpuRegisters, _instr: u16) -> u32 {
    1
}
pub fn handle_load_address(_regs: &mut CpuRegisters, _instr: u16) -> u32 {
    1
}
pub fn handle_sp_offset(_regs: &mut CpuRegisters, _instr: u16) -> u32 {
    1
}
