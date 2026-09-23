use crate::cpu_registers::CpuRegisters;
use crate::memory::GbaMemoryBus;

pub fn handle(regs: &mut CpuRegisters, bus: &mut GbaMemoryBus, instr: u16) -> u32 {
    let l = (instr >> 11) & 1 != 0; // 0=PUSH, 1=POP
    let r = (instr >> 8) & 1 != 0; // PC/LR
    let rlist = instr & 0xFF;
    if l {
        // POP loads low registers in ascending order and optionally loads PC last.
        pop(regs, bus, rlist, r)
    } else {
        // PUSH stores low registers in ascending order and optionally stores LR last.
        push(regs, bus, rlist, r)
    }
}

fn push(regs: &mut CpuRegisters, bus: &mut GbaMemoryBus, list: u16, link: bool) -> u32 {
    let count = list.count_ones() + u32::from(link);
    let mut address = regs.sp().wrapping_sub(count * 4);
    regs.set_sp(address);
    for register in selected_registers(list) {
        bus.write32(address, regs.r(register));
        address = address.wrapping_add(4);
    }
    if link {
        bus.write32(address, regs.lr());
    }
    3 + count
}

fn pop(regs: &mut CpuRegisters, bus: &mut GbaMemoryBus, list: u16, pc: bool) -> u32 {
    let mut address = regs.sp();
    for register in selected_registers(list) {
        regs.set_r(register, bus.read32(address));
        address = address.wrapping_add(4);
    }
    if pc {
        regs.set_pc(bus.read32(address));
        address = address.wrapping_add(4);
    }
    regs.set_sp(address);
    3 + list.count_ones() + u32::from(pc)
}

fn selected_registers(list: u16) -> impl Iterator<Item = usize> {
    (0..8).filter(move |register| list & (1 << register) != 0)
}
