use crate::cpu::arm_opcodes::helpers::condition_passed;
use crate::cpu_registers::CpuRegisters;
use crate::memory::GbaMemoryBus;

pub fn handle_cond(regs: &mut CpuRegisters, instr: u16) -> u32 {
    let cond = ((instr >> 8) & 0xF) as u8;
    let offset = ((instr & 0xFF) as i8 as i32) << 1;
    if condition_passed(regs.cpsr(), cond) {
        let pc = regs.pc();
        regs.set_pc(pc.wrapping_add(offset as u32));
        3
    } else {
        1
    }
}

pub fn handle_uncond(regs: &mut CpuRegisters, instr: u16) -> u32 {
    let offset = ((instr & 0x7FF) as i32) << 1;
    let offset = (offset << 20) >> 20; // sign extend the shifted 12-bit offset
    let pc = regs.pc();
    regs.set_pc(pc.wrapping_add(offset as u32));
    3
}

pub fn handle_long_bl(regs: &mut CpuRegisters, instr: u16) -> u32 {
    let h = (instr >> 11) & 1;
    if h == 0 {
        // First half: offset11
        let offset = ((instr & 0x7FF) as i32) << 12;
        let offset = (offset << 9) >> 9; // sign extend
        let pc = regs.pc();
        let target = pc.wrapping_add(offset as u32);
        regs.set_lr(target);
        1
    } else {
        // Second half: target=LR+offset, return address=次命令|1
        let offset = ((instr & 0x7FF) as u32) << 1;
        let lr = regs.lr();
        let target = lr.wrapping_add(offset);
        let pc_next = regs.pc().wrapping_sub(2) | 1;
        regs.set_lr(pc_next);
        regs.set_pc(target & !1);
        3
    }
}

pub fn handle_swi(regs: &mut CpuRegisters, bus: &mut GbaMemoryBus, instr: u16) -> u32 {
    let swi = (instr & 0xFF) as u8;
    match crate::bios::handle_swi(regs, bus, swi) {
        crate::bios::SwiResult::Return(cycles) | crate::bios::SwiResult::Branch(cycles) => {
            return cycles;
        }
        crate::bios::SwiResult::Unsupported => {}
    }
    let return_address = regs.pc().wrapping_sub(2);
    regs.enter_exception(0x13, 0x08, return_address, true);
    3
}

pub fn handle_undefined(regs: &mut CpuRegisters) -> u32 {
    let return_address = regs.pc().wrapping_sub(2);
    regs.enter_exception(0x1B, 0x04, return_address, false);
    3
}

#[cfg(test)]
mod unconditional_tests {
    use super::*;

    #[test]
    fn supports_full_positive_and_negative_range() {
        let mut registers = CpuRegisters::post_bios();
        registers.set_cpsr(registers.cpsr() | (1 << 5));
        registers.set_pc(0x08003F08);
        handle_uncond(&mut registers, 0xE317);
        assert_eq!(registers.pc(), 0x08004536);

        registers.set_pc(0x08001000);
        handle_uncond(&mut registers, 0xE7FF);
        assert_eq!(registers.pc(), 0x08000FFE);
    }
}
