use crate::cpu::registers::CpuRegisters;
use crate::memory::GbaMemoryBus;

pub fn handle(regs: &mut CpuRegisters, _bus: &mut GbaMemoryBus, _instr: u32) -> u32 {
    let return_address = regs.pc().wrapping_sub(4);
    regs.enter_exception(0x13, 0x08, return_address, true);
    3
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn swi_enters_svc_with_banked_lr() {
        let mut regs = CpuRegisters::post_bios();
        regs.set_pc(0x08000008);
        let old_cpsr = regs.cpsr();
        let mut bus = GbaMemoryBus::new();
        handle(&mut regs, &mut bus, 0xEF000000);
        assert_eq!(regs.cpsr_mode(), 0x13);
        assert_eq!(regs.spsr(), old_cpsr);
        assert_eq!(regs.lr(), 0x08000004);
        assert_eq!(regs.pc(), 0x08);
    }
}
