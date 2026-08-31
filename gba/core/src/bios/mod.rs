pub mod decompress;

use crate::cpu_registers::CpuRegisters;
use crate::memory::GbaMemoryBus;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SwiResult {
    Return(u32),
    Branch(u32),
    Unsupported,
}

/// HLE BIOS dispatcher.
pub fn handle_swi(regs: &mut CpuRegisters, bus: &mut GbaMemoryBus, swi: u8) -> SwiResult {
    match swi {
        0x00 => {
            soft_reset(regs, bus);
            SwiResult::Branch(3)
        }
        0x01 => {
            register_ram_reset(regs, bus);
            SwiResult::Return(1)
        }
        0x02 => {
            halt(bus);
            SwiResult::Return(1)
        }
        0x04 => {
            intr_wait(regs, bus);
            SwiResult::Return(1)
        }
        0x05 => {
            vblank_intr_wait(regs, bus);
            SwiResult::Return(1)
        }
        0x06 => {
            div(regs);
            SwiResult::Return(10)
        }
        0x0B => SwiResult::Return(cpu_set(regs, bus)),
        0x0C => SwiResult::Return(cpu_fast_set(regs, bus)),
        0x0D => {
            bios_checksum(regs);
            SwiResult::Return(1)
        }
        0x10 => {
            decompress::bit_unpack(regs, bus);
            SwiResult::Return(6)
        }
        0x11 => {
            decompress::lz77(regs, bus, 1);
            SwiResult::Return(20)
        }
        0x12 => {
            decompress::lz77(regs, bus, 2);
            SwiResult::Return(25)
        }
        0x13 => {
            decompress::huff(regs, bus);
            SwiResult::Return(30)
        }
        0x14 => {
            decompress::rl(regs, bus, 1);
            SwiResult::Return(15)
        }
        0x15 => {
            decompress::rl(regs, bus, 2);
            SwiResult::Return(18)
        }
        _ => SwiResult::Unsupported,
    }
}

fn soft_reset(regs: &mut CpuRegisters, bus: &mut GbaMemoryBus) {
    let boot_from_ewram = bus.read8(0x03007FFA) != 0;
    for addr in (0x03007E00..0x03008000).step_by(4) {
        bus.write32(addr, 0);
    }
    regs.set_sp(0x03007FE0);
    regs.set_pc(if boot_from_ewram {
        0x02000000
    } else {
        0x08000000
    });
}

fn register_ram_reset(regs: &mut CpuRegisters, bus: &mut GbaMemoryBus) {
    let flags = regs.r(0) as u8;
    if flags & 1 != 0 {
        for addr in (0x02000000..0x02040000).step_by(4) {
            bus.write32(addr, 0);
        }
    }
    if flags & 2 != 0 {
        for addr in (0x03000000..0x03007F00).step_by(4) {
            bus.write32(addr, 0);
        }
        // Don't clear 0x03007F00-0x03007FFF (stack)
    }
    if flags & 4 != 0 {
        for addr in (0x05000000..0x05000400).step_by(4) {
            bus.write32(addr, 0);
        }
    }
    if flags & 8 != 0 {
        for addr in (0x06000000..0x06018000).step_by(4) {
            bus.write32(addr, 0);
        }
    }
    if flags & 16 != 0 {
        for addr in (0x07000000..0x07000400).step_by(4) {
            bus.write32(addr, 0);
        }
    }
    bus.reset_io_groups(flags);
    regs.set_r(0, 0);
}

fn halt(bus: &mut GbaMemoryBus) {
    bus.write8(0x04000301, 0x00);
    bus.enter_halt(0x3FFF);
}

fn intr_wait(regs: &mut CpuRegisters, bus: &mut GbaMemoryBus) {
    // r0=discard, r1=irqMask
    let discard = regs.r(0) & 1 != 0;
    let mask = regs.r(1) as u16;
    if discard {
        let bios_flags = bus.read16(0x03007FF8) & !mask;
        bus.write16(0x03007FF8, bios_flags);
        bus.write16(0x04000202, mask);
    }
    bus.write16(0x04000208, 1);
    bus.write8(0x04000301, 0);
    bus.enter_halt(mask);
}

fn vblank_intr_wait(_regs: &mut CpuRegisters, bus: &mut GbaMemoryBus) {
    let bios_flags = bus.read16(0x03007FF8) & !1;
    bus.write16(0x03007FF8, bios_flags);
    bus.write16(0x04000202, 1);
    bus.write16(0x04000208, 1);
    bus.write8(0x04000301, 0);
    bus.enter_halt(1);
}

fn div(regs: &mut CpuRegisters) {
    let num = regs.r(0) as i32;
    let den = regs.r(1) as i32;
    if den == 0 {
        regs.set_r(0, -1i32 as u32);
        regs.set_r(1, num as u32);
        regs.set_r(3, num.unsigned_abs());
    } else {
        let (quotient, overflow) = num.overflowing_div(den);
        let remainder = if overflow { 0 } else { num % den };
        regs.set_r(0, quotient as u32);
        regs.set_r(1, remainder as u32);
        regs.set_r(3, quotient.unsigned_abs());
    }
}

fn cpu_set(regs: &mut CpuRegisters, bus: &mut GbaMemoryBus) -> u32 {
    let src = regs.r(0);
    let dst = regs.r(1);
    let len_mode = regs.r(2);
    let count = len_mode & 0x1FFFFF;
    let fixed = (len_mode >> 24) & 1 != 0;
    let is32 = (len_mode >> 26) & 1 != 0;

    if bus.is_bios_addr(src) {
        return 1;
    }

    if is32 {
        let mut s = src & !3;
        let mut d = dst & !3;
        for _ in 0..count {
            let v = bus.read32(s);
            bus.write32(d, v);
            if !fixed {
                s = s.wrapping_add(4);
            }
            d = d.wrapping_add(4);
        }
    } else {
        let mut s = src & !1;
        let mut d = dst & !1;
        for _ in 0..count {
            let v = bus.read16(s) as u32;
            bus.write16(d, v as u16);
            if !fixed {
                s = s.wrapping_add(2);
            }
            d = d.wrapping_add(2);
        }
    }
    1 + count
}

fn cpu_fast_set(regs: &mut CpuRegisters, bus: &mut GbaMemoryBus) -> u32 {
    let src = regs.r(0);
    let dst = regs.r(1);
    let len_mode = regs.r(2);
    let mut count = len_mode & 0x1FFFFF;
    let fixed = (len_mode >> 24) & 1 != 0;

    if bus.is_bios_addr(src) {
        return 1;
    }

    // FastSet rounds up to 8 words (32 bytes)
    count = (count + 7) & !7;
    let mut s = src & !3;
    let mut d = dst & !3;
    for _ in (0..count).step_by(8) {
        for _ in 0..8 {
            let v = bus.read32(s);
            bus.write32(d, v);
            if !fixed {
                s = s.wrapping_add(4);
            }
            d = d.wrapping_add(4);
        }
    }
    1 + count
}

fn bios_checksum(regs: &mut CpuRegisters) {
    // Simple checksum of BIOS 0x00000000-0x03FFF words sum
    // For HLE, return fixed value that matches BIOS
    regs.set_r(0, 0xBAAE187F);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn soft_reset_clears_iwram_and_branches() {
        let mut regs = CpuRegisters::post_bios();
        let mut bus = GbaMemoryBus::new();
        bus.write32(0x03007E00, 0xDEADBEEF);
        assert_eq!(handle_swi(&mut regs, &mut bus, 0), SwiResult::Branch(3));
        assert_eq!(regs.pc(), 0x08000000);
        assert_eq!(regs.sp(), 0x03007FE0);
        assert_eq!(bus.read32(0x03007E00), 0);
    }

    #[test]
    fn soft_reset_can_boot_from_ewram() {
        let mut regs = CpuRegisters::post_bios();
        let mut bus = GbaMemoryBus::new();
        bus.write8(0x03007FFA, 1);
        handle_swi(&mut regs, &mut bus, 0);
        assert_eq!(regs.pc(), 0x02000000);
    }

    #[test]
    fn div_handles_minimum_without_panicking() {
        let mut regs = CpuRegisters::post_bios();
        let mut bus = GbaMemoryBus::new();
        regs.set_r(0, i32::MIN as u32);
        regs.set_r(1, -1i32 as u32);
        handle_swi(&mut regs, &mut bus, 6);
        assert_eq!(regs.r(0), i32::MIN as u32);
        assert_eq!(regs.r(1), 0);
        assert_eq!(regs.r(3), 0x80000000);
    }

    #[test]
    fn div_by_zero_uses_documented_result() {
        let mut regs = CpuRegisters::post_bios();
        let mut bus = GbaMemoryBus::new();
        regs.set_r(0, -7i32 as u32);
        regs.set_r(1, 0);
        handle_swi(&mut regs, &mut bus, 6);
        assert_eq!(regs.r(0), u32::MAX);
        assert_eq!(regs.r(1), -7i32 as u32);
        assert_eq!(regs.r(3), 7);
    }

    #[test]
    fn cpu_set_copies_and_fills() {
        let mut regs = CpuRegisters::post_bios();
        let mut bus = GbaMemoryBus::new();
        bus.write32(0x02000000, 0x12345678);
        regs.set_r(0, 0x02000000);
        regs.set_r(1, 0x03000000);
        regs.set_r(2, (1 << 26) | 1);
        handle_swi(&mut regs, &mut bus, 0x0B);
        assert_eq!(bus.read32(0x03000000), 0x12345678);

        regs.set_r(1, 0x03000004);
        regs.set_r(2, (1 << 26) | (1 << 24) | 2);
        handle_swi(&mut regs, &mut bus, 0x0B);
        assert_eq!(bus.read32(0x03000004), 0x12345678);
        assert_eq!(bus.read32(0x03000008), 0x12345678);
    }

    #[test]
    fn halt_waits_for_enabled_interrupt() {
        let mut regs = CpuRegisters::post_bios();
        let mut bus = GbaMemoryBus::new();
        bus.write16(0x04000200, 1);
        handle_swi(&mut regs, &mut bus, 2);
        assert!(bus.is_halted());
        bus.request_interrupt(1);
        assert!(!bus.is_halted());
    }

    #[test]
    fn intr_wait_discards_only_requested_flags() {
        let mut regs = CpuRegisters::post_bios();
        let mut bus = GbaMemoryBus::new();
        bus.write16(0x04000200, 3);
        bus.request_interrupt(3);
        regs.set_r(0, 1);
        regs.set_r(1, 1);
        handle_swi(&mut regs, &mut bus, 4);
        assert_eq!(bus.read16(0x04000202), 2);
        assert_eq!(bus.read16(0x03007FF8), 2);
        assert!(bus.is_halted());
    }
}
