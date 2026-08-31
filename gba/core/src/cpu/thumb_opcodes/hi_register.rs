use crate::cpu::registers::CpuRegisters;
use crate::memory::GbaMemoryBus;

pub fn handle(regs: &mut CpuRegisters, _bus: &mut GbaMemoryBus, instr: u16) -> u32 {
    let op = (instr >> 8) & 0b11;
    let h1 = (instr >> 7) & 1 != 0;
    let h2 = (instr >> 6) & 1 != 0;
    let rs = ((instr >> 3) & 0x7) as usize + if h1 { 8 } else { 0 };
    let rd = (instr & 0x7) as usize + if h2 { 8 } else { 0 };
    match op {
        0b00 => {
            // ADD Rd, Rs
            let v = regs.r(rd).wrapping_add(regs.r(rs));
            regs.set_r(rd, v);
            if rd == 15 {
                regs.set_cpsr(regs.cpsr() & !(1 << 5)); // stay Thumb? BX handles T
            }
            1
        }
        0b01 => {
            // CMP Rd, Rs
            let a = regs.r(rd);
            let b = regs.r(rs);
            let (r, _) = a.overflowing_sub(b);
            regs.set_cpsr_n((r >> 31) & 1 != 0);
            regs.set_cpsr_z(r == 0);
            regs.set_cpsr_c(a >= b);
            regs.set_cpsr_v(((a ^ b) & (a ^ r) & 0x80000000) != 0);
            1
        }
        0b10 => {
            // MOV Rd, Rs
            let v = regs.r(rs);
            regs.set_r(rd, v);
            if rd == 15 {
                // MOV PC, Rs may switch via bit0? In Thumb, MOV PC doesn't switch T via bit0? Actually BX does.
            }
            1
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
