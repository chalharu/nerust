use crate::cpu_registers::CpuRegisters;
use crate::memory::GbaMemoryBus;

fn check_cond(cpsr: u32, cond: u8) -> bool {
    let n = cpsr & (1 << 31) != 0;
    let z = cpsr & (1 << 30) != 0;
    let c = cpsr & (1 << 29) != 0;
    let v = cpsr & (1 << 28) != 0;
    match cond {
        0x0 => z,
        0x1 => !z,
        0x2 => c,
        0x3 => !c,
        0x4 => n,
        0x5 => !n,
        0x6 => v,
        0x7 => !v,
        0x8 => c && !z,
        0x9 => !c || z,
        0xA => n == v,
        0xB => n != v,
        0xC => !z && n == v,
        0xD => z || n != v,
        _ => false,
    }
}

pub fn handle_cond(regs: &mut CpuRegisters, instr: u16) -> u32 {
    let cond = ((instr >> 8) & 0xF) as u8;
    let offset = ((instr & 0xFF) as i8 as i32) << 1;
    if check_cond(regs.cpsr(), cond) {
        let pc = regs.pc();
        regs.set_pc(pc.wrapping_add(offset as u32));
        3
    } else {
        1
    }
}

pub fn handle_uncond(regs: &mut CpuRegisters, instr: u16) -> u32 {
    let offset = ((instr & 0x7FF) as i32) << 1;
    let offset = (offset << 21) >> 21; // sign extend 11 bits
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
    if let Some(cycles) = crate::bios::handle_swi(regs, bus, swi) {
        return cycles;
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
