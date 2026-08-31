use crate::cpu_registers::CpuRegisters;
use crate::memory::GbaMemoryBus;

pub fn handle(regs: &mut CpuRegisters, bus: &mut GbaMemoryBus, instr: u32) -> u32 {
    let swi = ((instr >> 16) & 0xFF) as u8;
    match crate::bios::handle_swi(regs, bus, swi) {
        crate::bios::SwiResult::Return(cycles) | crate::bios::SwiResult::Branch(cycles) => {
            return cycles;
        }
        crate::bios::SwiResult::Unsupported => {}
    }
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
        handle(&mut regs, &mut bus, 0xEFFF0000);
        assert_eq!(regs.cpsr_mode(), 0x13);
        assert_eq!(regs.spsr(), old_cpsr);
        assert_eq!(regs.lr(), 0x08000004);
        assert_eq!(regs.pc(), 0x08);
    }

    #[test]
    fn arm_swi_uses_bits_23_16_for_hle_number() {
        let mut regs = CpuRegisters::post_bios();
        regs.set_pc(0x08000008);
        let mut bus = GbaMemoryBus::new();
        handle(&mut regs, &mut bus, 0xEF0D0000);
        assert_eq!(regs.r(0), 0xBAAE187F);
        assert_eq!(regs.cpsr_mode(), 0x1F);
    }
}
