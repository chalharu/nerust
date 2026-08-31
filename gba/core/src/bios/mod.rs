pub mod decompress;

use crate::cpu_registers::CpuRegisters;
use crate::memory::GbaMemoryBus;

/// HLE BIOS dispatcher. Returns Some(cycles) if handled, None if should fallback to exception.
pub fn handle_swi(regs: &mut CpuRegisters, bus: &mut GbaMemoryBus, swi: u8) -> Option<u32> {
    match swi {
        0x00 => {
            soft_reset(regs, bus);
            None // SoftReset doesn't return
        }
        0x01 => {
            register_ram_reset(regs, bus);
            Some(1)
        }
        0x02 => {
            halt(bus);
            Some(1)
        }
        0x04 => {
            intr_wait(regs, bus);
            Some(1)
        }
        0x05 => {
            vblank_intr_wait(regs, bus);
            Some(1)
        }
        0x06 => {
            div(regs);
            Some(10)
        }
        0x0B => {
            cpu_set(regs, bus);
            Some(5)
        }
        0x0C => {
            cpu_fast_set(regs, bus);
            Some(8)
        }
        0x0D => {
            bios_checksum(regs);
            Some(1)
        }
        0x10 => {
            decompress::bit_unpack(regs, bus);
            Some(6)
        }
        0x11 => {
            decompress::lz77(regs, bus, 1);
            Some(20)
        }
        0x12 => {
            decompress::lz77(regs, bus, 2);
            Some(25)
        }
        0x13 => {
            decompress::huff(regs, bus);
            Some(30)
        }
        0x14 => {
            decompress::rl(regs, bus, 1);
            Some(15)
        }
        0x15 => {
            decompress::rl(regs, bus, 2);
            Some(18)
        }
        _ => None,
    }
}

fn soft_reset(_regs: &mut CpuRegisters, _bus: &mut GbaMemoryBus) {
    // For HLE, just reset to post-BIOS state. Caller will handle via exception fallback.
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
    // SIO/Sound/Other flags are ignored for Phase 6 minimal
    regs.set_r(0, 0);
}

fn halt(bus: &mut GbaMemoryBus) {
    // Halt until IE&IF !=0
    // For HLE, just set HALTCNT and let tick handle it
    bus.write8(0x04000301, 0x00);
}

fn intr_wait(regs: &mut CpuRegisters, bus: &mut GbaMemoryBus) {
    // r0=discard, r1=irqMask
    let discard = regs.r(0) & 1 != 0;
    let _mask = regs.r(1);
    if discard {
        // Clear BIOS IRQ flags at 0x03007FF8
        bus.write32(0x03007FF8, 0);
        bus.write16(0x04000202, 0xFFFF); // clear IF
    }
    // Enable IME
    bus.write16(0x04000208, 1);
    // For HLE, just halt until interrupt
    halt(bus);
}

fn vblank_intr_wait(_regs: &mut CpuRegisters, bus: &mut GbaMemoryBus) {
    // VBlankIntrWait is IntrWait(1,1)
    bus.write32(0x03007FF8, 0);
    bus.write16(0x04000208, 1);
    halt(bus);
}

fn div(regs: &mut CpuRegisters) {
    let num = regs.r(0) as i32;
    let den = regs.r(1) as i32;
    if den == 0 {
        regs.set_r(0, if num >= 0 { -1i32 as u32 } else { 1 });
        regs.set_r(1, num as u32);
        regs.set_r(3, (num as i32).abs() as u32);
    } else {
        regs.set_r(0, (num / den) as u32);
        regs.set_r(1, (num % den) as u32);
        regs.set_r(3, ((num / den).abs()) as u32);
    }
}

fn cpu_set(regs: &mut CpuRegisters, bus: &mut GbaMemoryBus) {
    let src = regs.r(0);
    let dst = regs.r(1);
    let len_mode = regs.r(2);
    let count = len_mode & 0x1FFFFF;
    let fixed = (len_mode >> 24) & 1 != 0;
    let is32 = (len_mode >> 26) & 1 != 0;

    if bus.is_bios_addr(src) {
        return;
    }

    if is32 {
        let mut s = src;
        let mut d = dst;
        for _ in 0..count {
            let v = bus.read32(s);
            bus.write32(d, v);
            if !fixed {
                s = s.wrapping_add(4);
            }
            d = d.wrapping_add(4);
        }
    } else {
        let mut s = src;
        let mut d = dst;
        for _ in 0..count {
            let v = bus.read16(s) as u32;
            bus.write16(d, v as u16);
            if !fixed {
                s = s.wrapping_add(2);
            }
            d = d.wrapping_add(2);
        }
    }
}

fn cpu_fast_set(regs: &mut CpuRegisters, bus: &mut GbaMemoryBus) {
    let src = regs.r(0);
    let dst = regs.r(1);
    let len_mode = regs.r(2);
    let mut count = len_mode & 0x1FFFFF;
    let fixed = (len_mode >> 24) & 1 != 0;

    if bus.is_bios_addr(src) {
        return;
    }

    // FastSet rounds up to 8 words (32 bytes)
    count = (count + 7) & !7;
    let mut s = src;
    let mut d = dst;
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
}

fn bios_checksum(regs: &mut CpuRegisters) {
    // Simple checksum of BIOS 0x00000000-0x03FFF words sum
    // For HLE, return fixed value that matches BIOS
    regs.set_r(0, 0xBAAE187F);
}
