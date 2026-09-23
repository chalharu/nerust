use crate::cpu_registers::CpuRegisters;
use crate::memory::GbaMemoryBus;

/// SWP/SWPB (GBATEK ARM Single Data Swap): atomic load-then-store with the
/// HW bus lock (no DMA between the pair, so micro-op expansion keeps it a
/// single commit). Rd == R15 is UNPREDICTABLE: skip the write like MUL.
pub fn handle(regs: &mut CpuRegisters, bus: &mut GbaMemoryBus, instr: u32) -> u32 {
    let b = (instr >> 22) & 1 != 0;
    let rn = ((instr >> 16) & 0xF) as usize;
    let rd = ((instr >> 12) & 0xF) as usize;
    let rm = (instr & 0xF) as usize;
    let addr = regs.r(rn);
    let rm_val = regs.r(rm);
    let mem_val = if b {
        bus.read8(addr) as u32
    } else {
        bus.read32(addr)
    };
    if b {
        bus.write8(addr, (rm_val & 0xFF) as u8);
    } else {
        bus.write32(addr, rm_val);
    }
    // Rd == R15 is UNPREDICTABLE (ARM ARM): skip the write like MUL.
    if rd != 15 {
        regs.set_r(rd, mem_val);
    }
    // Load+store breaks the fetch stream (once per instruction).
    bus.charge_fetch_stream_break(addr);
    4
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn swp_exchanges_word() {
        let mut regs = CpuRegisters::post_bios();
        let mut bus = GbaMemoryBus::new();
        bus.write32(0x03000000, 0x11223344);
        regs.set_r(0, 0x03000000);
        regs.set_r(1, 0xAABBCCDD);
        // SWP R2, R1, [R0]
        handle(&mut regs, &mut bus, 0xE1002091);
        assert_eq!(regs.r(2), 0x11223344);
        assert_eq!(bus.read32(0x03000000), 0xAABBCCDD);
    }

    #[test]
    fn swpb_exchanges_byte() {
        let mut regs = CpuRegisters::post_bios();
        let mut bus = GbaMemoryBus::new();
        bus.write8(0x03000000, 0x44);
        regs.set_r(0, 0x03000000);
        regs.set_r(1, 0xDD);
        // SWPB R2, R1, [R0]
        handle(&mut regs, &mut bus, 0xE1402091);
        assert_eq!(regs.r(2), 0x44);
        assert_eq!(bus.read8(0x03000000), 0xDD);
    }
}
