use crate::cpu::registers::CpuRegisters;
use crate::memory::GbaMemoryBus;

pub fn handle(regs: &mut CpuRegisters, _bus: &mut GbaMemoryBus, instr: u32) -> u32 {
    let l = (instr >> 24) & 1 != 0;
    let offset = ((instr & 0xFFFFFF) << 2) as i32;
    // Sign extend 24-bit offset
    let offset = (offset << 6) >> 6;
    let pc = regs.pc();
    let target = pc.wrapping_add(offset as u32);
    if l {
        regs.set_lr(pc.wrapping_sub(4));
    }
    regs.set_pc(target);
    3
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cpu::registers::CpuRegisters;
    use crate::memory::GbaMemoryBus;

    #[test]
    fn b_forward() {
        let mut regs = CpuRegisters::post_bios();
        let mut bus = GbaMemoryBus::new();
        regs.set_pc(0x08000000);
        // Architectural PC is current instruction + 8.
        let b = 0xEA000002u32;
        handle(&mut regs, &mut bus, b);
        assert_eq!(regs.pc(), 0x08000008);
    }
}
