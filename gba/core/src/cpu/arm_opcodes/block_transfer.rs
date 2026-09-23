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
    if reg_list == 0 {
        return handle_empty_list(
            regs,
            bus,
            EmptyTransferSpec {
                base,
                base_register: rn,
                pre: p,
                up: u,
                writeback: w,
                load: l,
            },
        );
    }
    // P/U select IA, IB, DA, or DB; transfers still visit registers ascending.
    let start = start_address(base, reg_list.count_ones(), p, u);
    let writeback_value = if u {
        base.wrapping_add(reg_list.count_ones() * 4)
    } else {
        base.wrapping_sub(reg_list.count_ones() * 4)
    };
    let stored_base = (!l && reg_list & (1 << rn) != 0 && rn != reg_list.trailing_zeros() as usize)
        .then_some((rn, writeback_value));
    let transfer_user_bank = s && !(l && reg_list & (1 << 15) != 0);
    let transferred = transfer_registers(
        regs,
        bus,
        TransferSpec {
            list: reg_list,
            start,
            load: l,
            restore_cpsr: s,
            user_bank: transfer_user_bank,
            stored_base,
        },
    );

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

struct TransferSpec {
    list: u32,
    start: u32,
    load: bool,
    restore_cpsr: bool,
    user_bank: bool,
    stored_base: Option<(usize, u32)>,
}

struct EmptyTransferSpec {
    base: u32,
    base_register: usize,
    pre: bool,
    up: bool,
    writeback: bool,
    load: bool,
}

fn start_address(base: u32, count: u32, pre: bool, up: bool) -> u32 {
    match (up, pre) {
        (true, true) => base.wrapping_add(4),
        (true, false) => base,
        (false, true) => base.wrapping_sub(count * 4),
        (false, false) => base.wrapping_sub(count * 4).wrapping_add(4),
    }
}

fn transfer_registers(regs: &mut CpuRegisters, bus: &mut GbaMemoryBus, spec: TransferSpec) -> u32 {
    let mut address = spec.start;
    let mut transferred = 0;
    for register in (0..16).filter(|register| spec.list & (1 << register) != 0) {
        if spec.load {
            load_register(
                regs,
                bus,
                register,
                address,
                spec.restore_cpsr,
                spec.user_bank,
            );
        } else {
            store_register(
                regs,
                bus,
                register,
                address,
                spec.user_bank,
                spec.stored_base,
            );
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
    user_bank: bool,
) {
    let value = bus.read_aligned32(address);
    if user_bank {
        regs.set_user_r(register, value);
    } else {
        regs.set_r(register, value);
    }
    if restore && register == 15 {
        // LDM^ including PC returns from an exception and restores CPSR from SPSR.
        regs.set_cpsr(regs.spsr());
    }
}

fn store_register(
    regs: &CpuRegisters,
    bus: &mut GbaMemoryBus,
    register: usize,
    address: u32,
    user_bank: bool,
    stored_base: Option<(usize, u32)>,
) {
    // The architectural PC is instruction+8; STM stores instruction+12.
    let value = stored_base
        .filter(|(base_register, _)| *base_register == register)
        .map_or_else(
            || {
                if user_bank {
                    regs.user_r(register)
                } else {
                    regs.r(register)
                }
            },
            |(_, value)| value,
        )
        .wrapping_add(if register == 15 { 4 } else { 0 });
    bus.write32(address, value);
}

fn handle_empty_list(
    regs: &mut CpuRegisters,
    bus: &mut GbaMemoryBus,
    spec: EmptyTransferSpec,
) -> u32 {
    let address = start_address(spec.base, 16, spec.pre, spec.up);
    if spec.load {
        let target = bus.read_aligned32(address);
        if spec.writeback {
            regs.set_r(
                spec.base_register,
                if spec.up {
                    spec.base.wrapping_add(0x40)
                } else {
                    spec.base.wrapping_sub(0x40)
                },
            );
        }
        regs.set_pc(target);
        5
    } else {
        bus.write32(address, regs.pc().wrapping_add(4));
        if spec.writeback {
            regs.set_r(
                spec.base_register,
                if spec.up {
                    spec.base.wrapping_add(0x40)
                } else {
                    spec.base.wrapping_sub(0x40)
                },
            );
        }
        18
    }
}

fn transfer_cycles(load: bool, list: u32, transferred: u32) -> u32 {
    // Loading PC also incurs the pipeline refill cost.
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

    #[test]
    fn unaligned_base_aligns_memory_but_preserves_writeback_low_bits() {
        let mut regs = CpuRegisters::post_bios();
        let mut bus = GbaMemoryBus::new();
        let base = 0x02000100;
        regs.set_r(0, 32);
        regs.set_r(1, 64);
        regs.set_r(2, base + 3);
        regs.set_r(3, base - 5);

        handle(&mut regs, &mut bus, 0xE9220003); // STMDB R2!,{R0,R1}
        handle(&mut regs, &mut bus, 0xE8930030); // LDMIA R3,{R4,R5}
        assert_eq!(regs.r(4), 32);
        assert_eq!(regs.r(5), 64);
        assert_eq!(regs.r(2), regs.r(3));
    }
}
