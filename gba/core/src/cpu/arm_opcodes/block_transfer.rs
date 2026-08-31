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

    let base = regs.r(rn);
    let start = start_address(base, reg_list.count_ones(), p, u);
    let transferred = transfer_registers(regs, bus, reg_list, start, l, s);

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

    transfer_cycles(l, reg_list, transferred)
}

fn start_address(base: u32, count: u32, pre: bool, up: bool) -> u32 {
    match (up, pre) {
        (true, true) => base.wrapping_add(4),
        (true, false) => base,
        (false, true) => base.wrapping_sub(count * 4),
        (false, false) => base.wrapping_sub(count * 4).wrapping_add(4),
    }
}

fn transfer_registers(
    regs: &mut CpuRegisters,
    bus: &mut GbaMemoryBus,
    list: u32,
    mut address: u32,
    load: bool,
    user: bool,
) -> u32 {
    let mut transferred = 0;
    for register in (0..16).filter(|register| list & (1 << register) != 0) {
        if load {
            load_register(regs, bus, register, address, user);
        } else {
            store_register(regs, bus, register, address);
        }
        address = address.wrapping_add(4);
        transferred += 1;
    }
    transferred
}

fn load_register(
    regs: &mut CpuRegisters,
    bus: &mut GbaMemoryBus,
    register: usize,
    address: u32,
    restore: bool,
) {
    regs.set_r(register, bus.read32(address));
    if restore && register == 15 {
        regs.set_cpsr(regs.spsr());
    }
}

fn store_register(regs: &CpuRegisters, bus: &mut GbaMemoryBus, register: usize, address: u32) {
    // The architectural PC is instruction+8; STM stores instruction+12.
    let value = regs
        .r(register)
        .wrapping_add(if register == 15 { 4 } else { 0 });
    bus.write32(address, value);
}

fn transfer_cycles(load: bool, list: u32, transferred: u32) -> u32 {
    if load && list & (1 << 15) != 0 {
        5
    } else {
        2 + transferred
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
