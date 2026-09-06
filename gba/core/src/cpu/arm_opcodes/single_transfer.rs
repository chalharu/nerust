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
        // Register offsets use the immediate form of the barrel shifter.
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

    let (addr, wb_addr) =
        crate::cpu::arm_opcodes::helpers::transfer_addresses(regs.r(rn), offset, p, u);
    // Post-indexed transfers always write back; pre-indexed transfers use W.
    let writeback = w || !p;

    if l {
        // Writing R15 sets the PC-written latch and causes the pipeline to refill.
        regs.set_r(rd, load(bus, addr, b));
    } else {
        let value = regs.r(rd).wrapping_add(u32::from(rd == 15) * 4);
        store(bus, addr, value, b);
    }

    if writeback && !(l && rd == rn) {
        // Avoid writeback when Rd == Rn for LDR (UNPREDICTABLE)
        regs.set_r(rn, wb_addr);
    }

    if l { 3 } else { 2 }
}

fn load(bus: &mut GbaMemoryBus, address: u32, byte: bool) -> u32 {
    if byte {
        u32::from(bus.read8(address))
    } else {
        bus.read32(address)
    }
}

fn store(bus: &mut GbaMemoryBus, address: u32, value: u32, byte: bool) {
    if byte {
        bus.write8(address, value as u8);
    } else {
        bus.write32(address, value);
    }
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
