use crate::cpu_registers::CpuRegisters;
use crate::memory::GbaMemoryBus;

pub fn handle(regs: &mut CpuRegisters, _bus: &mut GbaMemoryBus, instr: u16) -> u32 {
    let op = (instr >> 8) & 0b11;
    let high_destination = (instr >> 7) & 1 != 0;
    let high_source = (instr >> 6) & 1 != 0;
    let rs = ((instr >> 3) & 0x7) as usize + if high_source { 8 } else { 0 };
    let rd = (instr & 0x7) as usize + if high_destination { 8 } else { 0 };
    match op {
        0b00 => {
            // ADD Rd, Rs (a write to PC is a branch: 2S+1N like BX)
            let v = regs.r(rd).wrapping_add(regs.r(rs));
            regs.set_r(rd, v);
            if rd == 15 { 3 } else { 1 }
        }
        0b01 => {
            // CMP Rd, Rs
            let a = regs.r(rd);
            let b = regs.r(rs);
            let (r, _) = a.overflowing_sub(b);
            crate::cpu::arm_opcodes::helpers::update_nz(regs, r);
            regs.set_cpsr_c(a >= b);
            regs.set_cpsr_v(((a ^ b) & (a ^ r) & 0x80000000) != 0);
            1
        }
        0b10 => {
            // MOV Rd, Rs (a write to PC is a branch: 2S+1N like BX;
            // Thumb MOV PC does not interwork, unlike BX)
            let v = regs.r(rs);
            regs.set_r(rd, v);
            if rd == 15 { 3 } else { 1 }
        }
        0b11 => {
            // BX Rs
            let target = regs.r(rs);
            let thumb = target & 1 != 0;
            regs.set_cpsr((regs.cpsr() & !(1 << 5)) | ((thumb as u32) << 5));
            regs.set_pc(target & !1);
            3
        }
        _ => 1,
    }
}
