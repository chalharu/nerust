use crate::cpu::registers::CpuRegisters;
use crate::memory::GbaMemoryBus;
pub fn handle(_regs: &mut CpuRegisters, _bus: &mut GbaMemoryBus, _instr: u16) -> u32 {
    1
}
