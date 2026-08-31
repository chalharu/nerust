use crate::cpu_registers::CpuRegisters;
use crate::memory::GbaMemoryBus;

pub fn handle(regs: &mut CpuRegisters, bus: &mut GbaMemoryBus, instr: u32) -> u32 {
    let p = (instr >> 24) & 1 != 0;
    let u = (instr >> 23) & 1 != 0;
    let s = (instr >> 22) & 1 != 0;
    let w = (instr >> 21) & 1 != 0;
    let l = (instr >> 20) & 1 != 0;
    let rn = ((instr >> 16) & 0xF) as usize;
    let reg_list = instr & 0xFFFF;

    let mut addr = regs.r(rn);
    let base = addr;
    // Calculate start address based on P/U
    if !u {
        // Decrement
        let count = reg_list.count_ones() * 4;
        if p {
            addr = addr.wrapping_sub(count);
        } else {
            addr = addr.wrapping_sub(count).wrapping_add(4);
        }
    } else if p {
        addr = addr.wrapping_add(4);
    }

    let mut transferred = 0;
    for i in 0..16 {
        if (reg_list >> i) & 1 != 0 {
            if l {
                let val = bus.read32(addr);
                regs.set_r(i, val);
                if s && i == 15 {
                    // LDM ^ with PC: CPSR = SPSR
                    let spsr = regs.spsr();
                    regs.set_cpsr(spsr);
                }
            } else {
                let mut val = regs.r(i);
                if i == 15 {
                    // 実行中命令+8のarchitectural PCに4を加え、PC+12を格納する。
                    val += 4;
                }
                bus.write32(addr, val);
            }
            addr = addr.wrapping_add(4);
            transferred += 1;
        }
    }

    if w {
        let wb_val = if u {
            base.wrapping_add(transferred * 4)
        } else {
            base.wrapping_sub(transferred * 4)
        };
        // Writeback not allowed if base in list and L==1 (UNPREDICTABLE)
        let base_in_list = (reg_list >> rn) & 1 != 0;
        if !(l && base_in_list) {
            regs.set_r(rn, wb_val);
        }
    }

    let _ = s;
    // LDM with PC takes additional cycles
    if l && (reg_list & (1 << 15)) != 0 {
        5
    } else if transferred > 0 {
        2 + transferred
    } else {
        2
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cpu_registers::CpuRegisters;
    use crate::memory::GbaMemoryBus;

    #[test]
    fn stm_ldm_roundtrip() {
        let mut regs = CpuRegisters::post_bios();
        let mut bus = GbaMemoryBus::new();
        regs.set_r(0, 0x02000000);
        regs.set_r(1, 0x11111111);
        regs.set_r(2, 0x22222222);
        // STMIA R0!, {R1,R2} -> E8A00006
        let stm = 0xE8A00006u32;
        handle(&mut regs, &mut bus, stm);
        // LDMIA R0!, {R3,R4} -> E8B10018 (but R0 already incremented)
        regs.set_r(0, 0x02000000);
        regs.set_r(3, 0);
        regs.set_r(4, 0);
        let ldm = 0xE8B00018u32; // LDMIA R0, {R3,R4}
        handle(&mut regs, &mut bus, ldm);
        assert_eq!(regs.r(3), 0x11111111);
        assert_eq!(regs.r(4), 0x22222222);
    }
}
