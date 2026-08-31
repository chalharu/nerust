use crate::cpu_registers::CpuRegisters;
use crate::memory::GbaMemoryBus;

pub fn handle_pc_relative(regs: &mut CpuRegisters, bus: &mut GbaMemoryBus, instr: u16) -> u32 {
    let rd = ((instr >> 8) & 0x7) as usize;
    let imm = ((instr & 0xFF) as u32) << 2;
    let addr = (regs.pc() & !3).wrapping_add(imm);
    let val = bus.read32(addr);
    regs.set_r(rd, val);
    3
}

pub fn handle_reg_offset(regs: &mut CpuRegisters, bus: &mut GbaMemoryBus, instr: u16) -> u32 {
    let ro = ((instr >> 6) & 0x7) as usize;
    let rb = ((instr >> 3) & 0x7) as usize;
    let rd = (instr & 0x7) as usize;
    let b = (instr >> 10) & 1 != 0;
    let l = (instr >> 11) & 1 != 0;
    let offset = regs.r(ro);
    let addr = regs.r(rb).wrapping_add(offset);
    if l {
        let val = if b {
            bus.read8(addr) as u32
        } else {
            bus.read32(addr)
        };
        regs.set_r(rd, val);
        3
    } else {
        let val = regs.r(rd);
        if b {
            bus.write8(addr, val as u8);
        } else {
            bus.write32(addr, val);
        }
        2
    }
}

pub fn handle_sign_extended(regs: &mut CpuRegisters, bus: &mut GbaMemoryBus, instr: u16) -> u32 {
    let op = (instr >> 10) & 0b11;
    let ro = ((instr >> 6) & 0x7) as usize;
    let rb = ((instr >> 3) & 0x7) as usize;
    let rd = (instr & 0x7) as usize;
    let addr = regs.r(rb).wrapping_add(regs.r(ro));
    match op {
        0b00 => {
            bus.write16(addr, regs.r(rd) as u16); // STRH
            2
        }
        0b01 => {
            regs.set_r(rd, bus.read8(addr) as i8 as i32 as u32); // LDRSB
            3
        }
        0b10 => {
            regs.set_r(rd, bus.read_ldr_halfword(addr)); // LDRH
            3
        }
        _ => {
            let value = if addr & 1 != 0 {
                bus.read8(addr) as i8 as i32 as u32
            } else {
                bus.read16(addr) as i16 as i32 as u32
            };
            regs.set_r(rd, value); // LDRSH
            3
        }
    }
}

pub fn handle_imm_offset(regs: &mut CpuRegisters, bus: &mut GbaMemoryBus, instr: u16) -> u32 {
    let b = (instr >> 12) & 1 != 0;
    let l = (instr >> 11) & 1 != 0;
    let offset = ((instr >> 6) & 0x1F) as u32;
    let rb = ((instr >> 3) & 0x7) as usize;
    let rd = (instr & 0x7) as usize;
    let addr = if b {
        regs.r(rb).wrapping_add(offset)
    } else {
        regs.r(rb).wrapping_add(offset << 2)
    };
    if l {
        let val = if b {
            bus.read8(addr) as u32
        } else {
            bus.read32(addr)
        };
        regs.set_r(rd, val);
        3
    } else {
        let val = regs.r(rd);
        if b {
            bus.write8(addr, val as u8);
        } else {
            bus.write32(addr, val);
        }
        2
    }
}

pub fn handle_halfword(regs: &mut CpuRegisters, bus: &mut GbaMemoryBus, instr: u16) -> u32 {
    let l = (instr >> 11) & 1 != 0;
    let offset = ((instr >> 6) & 0x1F) as u32;
    let rb = ((instr >> 3) & 0x7) as usize;
    let rd = (instr & 0x7) as usize;
    let addr = regs.r(rb).wrapping_add(offset << 1);
    if l {
        let val = bus.read_ldr_halfword(addr);
        regs.set_r(rd, val);
        3
    } else {
        bus.write16(addr, regs.r(rd) as u16);
        2
    }
}

pub fn handle_sp_relative(regs: &mut CpuRegisters, bus: &mut GbaMemoryBus, instr: u16) -> u32 {
    let l = (instr >> 11) & 1 != 0;
    let rd = ((instr >> 8) & 0x7) as usize;
    let imm = ((instr & 0xFF) as u32) << 2;
    let addr = regs.sp().wrapping_add(imm);
    if l {
        regs.set_r(rd, bus.read32(addr));
        3
    } else {
        bus.write32(addr, regs.r(rd));
        2
    }
}

pub fn handle_multiple(regs: &mut CpuRegisters, bus: &mut GbaMemoryBus, instr: u16) -> u32 {
    let l = (instr >> 11) & 1 != 0;
    let rb = ((instr >> 8) & 0x7) as usize;
    let rlist = instr & 0xFF;
    let mut addr = regs.r(rb);
    let mut count = 0;
    for i in 0..8 {
        if (rlist >> i) & 1 != 0 {
            if l {
                regs.set_r(i, bus.read32(addr));
            } else {
                bus.write32(addr, regs.r(i));
            }
            addr = addr.wrapping_add(4);
            count += 1;
        }
    }
    // Writeback if not in list
    if (rlist >> rb) & 1 == 0 {
        regs.set_r(rb, addr);
    }
    if l {
        3 + count as u32
    } else {
        2 + count as u32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signed_loads_extend_sign() {
        let mut regs = CpuRegisters::post_bios();
        let mut bus = GbaMemoryBus::new();
        regs.set_r(1, 0x03000000);
        regs.set_r(2, 0);
        bus.write16(0x03000000, 0x80FF);

        handle_sign_extended(&mut regs, &mut bus, 0x5688); // LDRSB R0,[R1,R2]
        assert_eq!(regs.r(0), 0xFFFFFFFF);
        handle_sign_extended(&mut regs, &mut bus, 0x5E88); // LDRSH R0,[R1,R2]
        assert_eq!(regs.r(0), 0xFFFF80FF);
    }
}
