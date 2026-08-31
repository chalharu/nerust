use crate::cpu::registers::CpuRegisters;
use crate::memory::GbaMemoryBus;
pub fn handle_cond(_regs: &mut CpuRegisters, _instr: u16) -> u32 {
    1
}
pub fn handle_swi(_regs: &mut CpuRegisters, _bus: &mut GbaMemoryBus, _instr: u16) -> u32 {
    1
}
pub fn handle_uncond(_regs: &mut CpuRegisters, _instr: u16) -> u32 {
    1
}
pub fn handle_long_bl(_regs: &mut CpuRegisters, _instr: u16) -> u32 {
    1
}
