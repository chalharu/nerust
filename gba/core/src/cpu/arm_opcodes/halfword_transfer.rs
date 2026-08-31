use crate::cpu::registers::CpuRegisters;
use crate::memory::GbaMemoryBus;

pub fn handle(regs: &mut CpuRegisters, bus: &mut GbaMemoryBus, instr: u32) -> u32 {
    let p = (instr >> 24) & 1 != 0;
    let u = (instr >> 23) & 1 != 0;
    let w = (instr >> 21) & 1 != 0;
    let l = (instr >> 20) & 1 != 0;
    let rn = ((instr >> 16) & 0xF) as usize;
    let rd = ((instr >> 12) & 0xF) as usize;
    let s = (instr >> 6) & 1 != 0;
    let h = (instr >> 5) & 1 != 0;

    // Offset: immediate or register
    let offset = if (instr >> 22) & 1 != 0 {
        // Immediate: offset = high*16 + low
        let high = (instr >> 8) & 0xF;
        let low = instr & 0xF;
        (high << 4) | low
    } else {
        let rm = (instr & 0xF) as usize;
        regs.r(rm)
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
    let wb_addr = if p {
        addr
    } else if u {
        base.wrapping_add(offset)
    } else {
        base.wrapping_sub(offset)
    };
    let writeback = w || !p;

    if l {
        let val = match (s, h) {
            (false, true) => bus.read16(addr) as u32,      // LDRH
            (true, false) => bus.read8(addr) as i8 as u32, // LDRSB
            (true, true) => {
                if addr & 1 != 0 {
                    bus.read8(addr) as i8 as i32 as u32
                } else {
                    bus.read16(addr) as i16 as i32 as u32
                }
            } // LDRSH
            _ => bus.read16(addr) as u32,
        };
        regs.set_r(rd, val);
    } else {
        let val = regs.r(rd);
        bus.write16(addr, (val & 0xFFFF) as u16);
    }

    if writeback && !(l && rd == rn) {
        regs.set_r(rn, wb_addr);
    }
    if l { 3 } else { 2 }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cpu::registers::CpuRegisters;
    use crate::memory::GbaMemoryBus;

    #[test]
    fn ldrh_strh() {
        let mut regs = CpuRegisters::post_bios();
        let mut bus = GbaMemoryBus::new();
        regs.set_r(1, 0x02000000);
        regs.set_r(0, 0x1234);
        // STRH R0, [R1] -> E181000B? simplified immediate 0
        // Use encoding: E1C100B0 = STRH R0, [R1]
        let strh = 0xE1C100B0u32;
        handle(&mut regs, &mut bus, strh);
        let ldrh = 0xE1D100B0u32; // LDRH R0, [R1]
        regs.set_r(0, 0);
        handle(&mut regs, &mut bus, ldrh);
        assert_eq!(regs.r(0) & 0xFFFF, 0x1234);
    }
}
