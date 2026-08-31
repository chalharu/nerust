use crate::cpu::registers::CpuRegisters;
use crate::memory::GbaMemoryBus;

pub fn handle(regs: &mut CpuRegisters, bus: &mut GbaMemoryBus, instr: u16) -> u32 {
    let l = (instr >> 11) & 1 != 0; // 0=PUSH, 1=POP
    let r = (instr >> 8) & 1 != 0; // PC/LR
    let rlist = instr & 0xFF;
    if !l {
        // PUSH {Rlist, LR}
        let mut count = rlist.count_ones() + if r { 1 } else { 0 };
        let mut addr = regs.sp().wrapping_sub(count * 4);
        regs.set_sp(addr);
        for i in 0..8 {
            if (rlist >> i) & 1 != 0 {
                bus.write32(addr, regs.r(i as usize));
                addr = addr.wrapping_add(4);
            }
        }
        if r {
            bus.write32(addr, regs.lr());
        }
        3 + count as u32
    } else {
        // POP {Rlist, PC}
        let mut addr = regs.sp();
        for i in 0..8 {
            if (rlist >> i) & 1 != 0 {
                regs.set_r(i as usize, bus.read32(addr));
                addr = addr.wrapping_add(4);
            }
        }
        if r {
            let val = bus.read32(addr);
            regs.set_pc(val & !1);
            addr = addr.wrapping_add(4);
        }
        regs.set_sp(addr);
        3 + rlist.count_ones() as u32 + if r { 1 } else { 0 }
    }
}
