use crate::cpu::registers::CpuRegisters;
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
        regs.set_pc(pc.wrapping_add(2).wrapping_add(offset as u32 + 2));
        3
    } else {
        1
    }
}

pub fn handle_uncond(regs: &mut CpuRegisters, instr: u16) -> u32 {
    let offset = ((instr & 0x7FF) as i32) << 1;
    let offset = ((offset << 21) >> 21) as i32; // sign extend 11 bits
    let pc = regs.pc();
    regs.set_pc(pc.wrapping_add(2).wrapping_add(offset as u32 + 2));
    3
}

pub fn handle_long_bl(regs: &mut CpuRegisters, instr: u16) -> u32 {
    let h = (instr >> 11) & 1;
    if h == 0 {
        // First half: offset11
        let offset = ((instr & 0x7FF) as i32) << 12;
        let offset = ((offset << 9) >> 9) as i32; // sign extend
        let pc = regs.pc();
        let target = pc.wrapping_add(2).wrapping_add(offset as u32);
        // Save to LR: PC+2 with bit0? Actually BL first half saves PC+2 + offset<<12 to LR
        regs.set_lr(target);
        1
    } else {
        // Second half: offset11, LR + offset<<1 +2, set PC, LR = PC+2|1
        let offset = ((instr & 0x7FF) as u32) << 1;
        let lr = regs.lr();
        let target = lr.wrapping_add(offset);
        let pc_next = regs.pc().wrapping_add(2) | 1;
        regs.set_lr(pc_next);
        regs.set_pc(target & !1);
        3
    }
}

pub fn handle_swi(regs: &mut CpuRegisters, bus: &mut GbaMemoryBus, instr: u16) -> u32 {
    let _ = bus;
    let imm = (instr & 0xFF) as u32;
    // SWI: SPSR_svc=CPSR, LR_svc=PC, CPSR=SVC|I, PC=0x08
    let cpsr = regs.cpsr();
    let pc = regs.pc();
    regs.set_r(14, pc.wrapping_add(2));
    // Switch to SVC
    regs.set_cpsr((cpsr & !(0x1F | (1 << 5))) | 0x13 | (1 << 7));
    regs.set_spsr(cpsr);
    regs.set_pc(0x08 + (imm & 0xFF) * 4); // HLE will intercept via SWI number
    3
}
