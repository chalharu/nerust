use crate::cpu_registers::CpuRegisters;
use crate::memory::GbaMemoryBus;

pub fn handle(regs: &mut CpuRegisters, bus: &mut GbaMemoryBus, instr: u32) -> u32 {
    let i = (instr >> 25) & 1 != 0;
    let p = (instr >> 24) & 1 != 0;
    let u = (instr >> 23) & 1 != 0;
    let b = (instr >> 22) & 1 != 0;
    let w = (instr >> 21) & 1 != 0;
    let l = (instr >> 20) & 1 != 0;
    let rn = ((instr >> 16) & 0xF) as usize;
    let rd = ((instr >> 12) & 0xF) as usize;

    let offset = if i {
        // Register offset with shift
        let rm = (instr & 0xF) as usize;
        let rm_val = regs.r(rm);
        let shift_type = ((instr >> 5) & 0b11) as u8;
        let shift_imm = (instr >> 7) & 0x1F;
        let (shifted, _) = crate::cpu::arm_opcodes::helpers::barrel_shift(
            rm_val,
            shift_type,
            shift_imm,
            regs.cpsr_c(),
        );
        shifted
    } else {
        instr & 0xFFF
    };

    let base = regs.r(rn);
    let addr = if p {
        if u {
            base.wrapping_add(offset)
        } else {
            base.wrapping_sub(offset)
        }
    } else {
        base
    };

    // Writeback if W or !P
    let writeback = w || !p;
    let wb_addr = if p {
        addr
    } else {
        if u {
            base.wrapping_add(offset)
        } else {
            base.wrapping_sub(offset)
        }
    };

    if l {
        // LDR
        let val = if b {
            bus.read8(addr) as u32
        } else {
            bus.read32(addr)
        };
        regs.set_r(rd, val);
        if rd == 15 {
            // LDR PC: pipeline flush handled by caller
        }
    } else {
        // STR
        let val = regs.r(rd);
        if b {
            bus.write8(addr, (val & 0xFF) as u8);
        } else {
            bus.write32(addr, val);
        }
    }

    if writeback && !(l && rd == rn) {
        // Avoid writeback when Rd == Rn for LDR (UNPREDICTABLE)
        regs.set_r(rn, wb_addr);
    }

    if l { 3 } else { 2 }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cpu_registers::CpuRegisters;
    use crate::memory::GbaMemoryBus;

    #[test]
    fn ldr_str_immediate() {
        let mut regs = CpuRegisters::post_bios();
        let mut bus = GbaMemoryBus::new();
        regs.set_r(1, 0x02000000);
        regs.set_r(0, 0x12345678);
        // STR R0, [R1, #4] -> E5810004
        let str_instr = 0xE5810004u32;
        handle(&mut regs, &mut bus, str_instr);
        // LDR R2, [R1, #4] -> E5912004
        let ldr_instr = 0xE5912004u32;
        handle(&mut regs, &mut bus, ldr_instr);
        assert_eq!(regs.r(2), 0x12345678);
    }
}
