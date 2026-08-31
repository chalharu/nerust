use crate::cpu::registers::CpuRegisters;
use crate::memory::GbaMemoryBus;

pub fn handle(regs: &mut CpuRegisters, _bus: &mut GbaMemoryBus, _instr: u32) -> u32 {
    // SWI exception: SPSR_svc = CPSR, LR_svc = PC+4, CPSR = SVC|I, PC=0x08
    let cpsr = regs.cpsr();
    // Save SPSR_svc (need to switch mode first? Simplified: set SPSR then mode)
    let pc = regs.pc();
    regs.set_r(14, pc.wrapping_add(4)); // LR_svc
    // Switch to SVC mode (0x13)
    regs.set_cpsr((cpsr & !(0x1F | (1 << 5))) | 0x13 | (1 << 7));
    // Set SPSR_svc = old CPSR
    regs.set_spsr(cpsr);
    regs.set_pc(0x08);
    3
}
