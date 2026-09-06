pub mod decompress;

use crate::cpu_registers::CpuRegisters;
use crate::memory::GbaMemoryBus;

const CPU_SET_SETUP_CYCLES: u32 = 61;
const CPU_SET_RETURN_CYCLES: u32 = 46;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SwiResult {
    Return(u32),
    Branch(u32),
    Unsupported,
}

pub(crate) struct HleBiosOperation {
    source: u32,
    destination: u32,
    remaining: u32,
    fixed: bool,
    width: u8,
    value: u32,
    phase: TransferPhase,
}

#[derive(Clone, Copy)]
enum TransferPhase {
    Setup(u32),
    Read,
    Write,
    Complete(u32),
}

pub(crate) struct HleStep {
    pub cycles: u32,
    pub complete: bool,
}

impl HleBiosOperation {
    fn cpu_set(source: u32, destination: u32, len_mode: u32) -> Option<Self> {
        let remaining = len_mode & 0x1F_FFFF;
        Self::transfer(source, destination, len_mode, remaining)
    }

    fn cpu_fast_set(source: u32, destination: u32, len_mode: u32) -> Option<Self> {
        let remaining = (len_mode & 0x1F_FFFF).next_multiple_of(8);
        Self::transfer(source, destination, len_mode | (1 << 26), remaining)
    }

    fn transfer(source: u32, destination: u32, len_mode: u32, remaining: u32) -> Option<Self> {
        if source < 0x0000_4000 || remaining == 0 {
            return None;
        }
        let width = if len_mode & (1 << 26) != 0 { 4 } else { 2 };
        Some(Self {
            source: source & !(u32::from(width) - 1),
            destination: destination & !(u32::from(width) - 1),
            remaining,
            fixed: len_mode & (1 << 24) != 0,
            width,
            value: 0,
            phase: TransferPhase::Setup(CPU_SET_SETUP_CYCLES),
        })
    }

    pub(crate) fn step(&mut self, bus: &mut GbaMemoryBus) -> HleStep {
        match self.phase {
            TransferPhase::Setup(remaining) => {
                self.phase = if remaining == 1 {
                    TransferPhase::Read
                } else {
                    TransferPhase::Setup(remaining - 1)
                };
                HleStep {
                    cycles: 1,
                    complete: false,
                }
            }
            TransferPhase::Read => {
                self.value = if self.width == 4 {
                    bus.read32(self.source)
                } else {
                    u32::from(bus.read16(self.source))
                };
                if !self.fixed {
                    self.source = self.source.wrapping_add(u32::from(self.width));
                }
                self.phase = TransferPhase::Write;
                HleStep {
                    cycles: 1,
                    complete: false,
                }
            }
            TransferPhase::Write => {
                if self.width == 4 {
                    bus.write_hle_bios32(self.destination, self.value);
                } else {
                    bus.write_hle_bios16(self.destination, self.value as u16);
                }
                self.destination = self.destination.wrapping_add(u32::from(self.width));
                self.remaining -= 1;
                self.phase = if self.remaining == 0 {
                    TransferPhase::Complete(CPU_SET_RETURN_CYCLES)
                } else {
                    TransferPhase::Read
                };
                HleStep {
                    cycles: 1,
                    complete: false,
                }
            }
            TransferPhase::Complete(remaining) => {
                self.phase = TransferPhase::Complete(remaining.saturating_sub(1));
                HleStep {
                    cycles: 1,
                    complete: remaining == 1,
                }
            }
        }
    }
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
        0x07 => {
            div_arm(regs);
            SwiResult::Return(10)
        }
        0x08 => {
            sqrt(regs);
            SwiResult::Return(10)
        }
        0x09 => {
            arc_tan(regs);
            SwiResult::Return(10)
        }
        0x0A => {
            arc_tan2(regs);
            SwiResult::Return(10)
        }
        0x0E => {
            bg_affine_set(regs, bus);
            SwiResult::Return(10)
        }
        0x0F => {
            obj_affine_set(regs, bus);
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
        0x03
        | 0x16
        | 0x17
        | 0x18
        | 0x19
        | 0x1A
        | 0x1B
        | 0x1C
        | 0x1D
        | 0x1E
        | 0x1F
        | 0x20..=0x2F => {
            // Sound / Diff / Stop / MultiBoot etc — no-op for HLE minimal
            SwiResult::Return(1)
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

fn div_arm(regs: &mut CpuRegisters) {
    // DivArm swaps r0 and r1 vs Div
    let den = regs.r(0) as i32;
    let num = regs.r(1) as i32;
    if den == 0 {
        regs.set_r(0, -1i32 as u32);
        regs.set_r(1, num as u32);
        regs.set_r(3, num.unsigned_abs());
    } else {
        let (q, o) = num.overflowing_div(den);
        let r = if o { 0 } else { num % den };
        regs.set_r(0, q as u32);
        regs.set_r(1, r as u32);
        regs.set_r(3, q.unsigned_abs());
    }
}

fn sqrt(regs: &mut CpuRegisters) {
    let n = regs.r(0);
    regs.set_r(0, n.isqrt());
}

fn arc_tan(regs: &mut CpuRegisters) {
    // GBATEK: r0 = Tan (1.14 fixed), return -PI/2..PI/2 => C000h..4000h
    let tan = regs.r(0) as i16 as f32 / 16384.0;
    let theta = tan.atan();
    let v = (theta * 65536.0 / (2.0 * std::f32::consts::PI)) as i32;
    regs.set_r(0, (v & 0xFFFF) as u32);
}

fn arc_tan2(regs: &mut CpuRegisters) {
    let x = regs.r(0) as i16 as f32 / 16384.0;
    let y = regs.r(1) as i16 as f32 / 16384.0;
    let theta = y.atan2(x);
    let mut v = (theta * 65536.0 / (2.0 * std::f32::consts::PI)) as i32;
    if v < 0 {
        v += 65536;
    }
    regs.set_r(0, (v & 0xFFFF) as u32);
}

fn bg_affine_set(regs: &mut CpuRegisters, bus: &mut GbaMemoryBus) {
    use crate::math::affine::{BgAffineDst, BgAffineSrc, bg_affine_set as math_bg};
    use crate::math::fixed_point::Fixed8_8;
    let src = regs.r(0);
    let dst = regs.r(1);
    let count = regs.r(2) as usize;
    for i in 0..count {
        let base_src = src + i as u32 * 20;
        let base_dst = dst + i as u32 * 16;
        let cx = bus.read32(base_src) as i32;
        let cy = bus.read32(base_src + 4) as i32;
        let disp_cx = bus.read16(base_src + 8) as i16;
        let disp_cy = bus.read16(base_src + 10) as i16;
        let sx = Fixed8_8::from_raw(bus.read16(base_src + 12) as i16);
        let sy = Fixed8_8::from_raw(bus.read16(base_src + 14) as i16);
        let alpha = bus.read16(base_src + 16);
        let s = BgAffineSrc {
            cx,
            cy,
            disp_cx,
            disp_cy,
            sx,
            sy,
            alpha,
        };
        let mut d = BgAffineDst {
            pa: Fixed8_8::from_raw(0),
            pb: Fixed8_8::from_raw(0),
            pc: Fixed8_8::from_raw(0),
            pd: Fixed8_8::from_raw(0),
            start_x: 0,
            start_y: 0,
        };
        math_bg(&s, &mut d);
        bus.write16(base_dst, d.pa.to_raw() as u16);
        bus.write16(base_dst + 2, d.pb.to_raw() as u16);
        bus.write16(base_dst + 4, d.pc.to_raw() as u16);
        bus.write16(base_dst + 6, d.pd.to_raw() as u16);
        bus.write32(base_dst + 8, d.start_x as u32);
        bus.write32(base_dst + 12, d.start_y as u32);
    }
}

fn obj_affine_set(regs: &mut CpuRegisters, bus: &mut GbaMemoryBus) {
    use crate::math::affine::{ObjAffineDst, ObjAffineSrc, obj_affine_set as math_obj};
    use crate::math::fixed_point::Fixed8_8;
    let src = regs.r(0);
    let dst = regs.r(1);
    let count = regs.r(2) as usize;
    let offset = regs.r(3) as usize;
    let mut base_dst = dst;
    for i in 0..count {
        let base_src = src + i as u32 * 8;
        let sx = Fixed8_8::from_raw(bus.read16(base_src) as i16);
        let sy = Fixed8_8::from_raw(bus.read16(base_src + 2) as i16);
        let alpha = bus.read16(base_src + 4);
        let s = ObjAffineSrc { sx, sy, alpha };
        let mut d = ObjAffineDst {
            pa: Fixed8_8::from_raw(0),
            pb: Fixed8_8::from_raw(0),
            pc: Fixed8_8::from_raw(0),
            pd: Fixed8_8::from_raw(0),
        };
        math_obj(&s, &mut d);
        bus.write16(base_dst, d.pa.to_raw() as u16);
        bus.write16(base_dst.wrapping_add(offset as u32), d.pb.to_raw() as u16);
        bus.write16(
            base_dst.wrapping_add(offset as u32 * 2),
            d.pc.to_raw() as u16,
        );
        bus.write16(
            base_dst.wrapping_add(offset as u32 * 3),
            d.pd.to_raw() as u16,
        );
        base_dst = base_dst.wrapping_add(offset as u32 * 4);
    }
}

fn cpu_set(regs: &mut CpuRegisters, bus: &mut GbaMemoryBus) -> u32 {
    let src = regs.r(0);
    let dst = regs.r(1);
    let len_mode = regs.r(2);
    if let Some(operation) = HleBiosOperation::cpu_set(src, dst, len_mode) {
        bus.start_hle_bios(operation);
    }
    1
}

fn cpu_fast_set(regs: &mut CpuRegisters, bus: &mut GbaMemoryBus) -> u32 {
    let src = regs.r(0);
    let dst = regs.r(1);
    let len_mode = regs.r(2);
    if let Some(operation) = HleBiosOperation::cpu_fast_set(src, dst, len_mode) {
        bus.start_hle_bios(operation);
    }
    1
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
        assert_eq!(bus.read32(0x03000000), 0);
        while bus.hle_bios_active() {
            bus.step_hle_bios();
        }
        assert_eq!(bus.read32(0x03000000), 0x12345678);

        regs.set_r(1, 0x03000004);
        regs.set_r(2, (1 << 26) | (1 << 24) | 2);
        handle_swi(&mut regs, &mut bus, 0x0B);
        while bus.hle_bios_active() {
            bus.step_hle_bios();
        }
        assert_eq!(bus.read32(0x03000004), 0x12345678);
        assert_eq!(bus.read32(0x03000008), 0x12345678);
    }

    #[test]
    fn cpu_fast_set_rounds_up_to_eight_words() {
        let mut regs = CpuRegisters::post_bios();
        let mut bus = GbaMemoryBus::new();
        for index in 0..8 {
            bus.write32(0x02000000 + index * 4, 0x1000 + index);
        }
        regs.set_r(0, 0x02000000);
        regs.set_r(1, 0x03000000);
        regs.set_r(2, 1);

        handle_swi(&mut regs, &mut bus, 0x0C);
        while bus.hle_bios_active() {
            bus.step_hle_bios();
        }

        for index in 0..8 {
            assert_eq!(bus.read32(0x03000000 + index * 4), 0x1000 + index);
        }
    }

    #[test]
    fn cpu_set_includes_bios_entry_and_return_cycles() {
        let mut bus = GbaMemoryBus::new();
        bus.write16(0x03000000, 0x1234);
        let mut operation = HleBiosOperation::cpu_set(0x03000000, 0x03000002, 1).unwrap();
        let mut cycles = 0;

        loop {
            let step = operation.step(&mut bus);
            cycles += step.cycles;
            if step.complete {
                break;
            }
        }

        assert_eq!(cycles, CPU_SET_SETUP_CYCLES + 2 + CPU_SET_RETURN_CYCLES);
        assert_eq!(bus.read16(0x03000002), 0x1234);
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
