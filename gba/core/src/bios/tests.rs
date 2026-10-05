use super::hle_operation::HleBiosOperation;
use super::compute::bios_arctan2_full;
use super::{handle_swi, test_bus, SwiResult};
use crate::cpu_registers::CpuRegisters;

#[test]
fn bg_affine_set_multi_entry_stride_is_18() {
    // GBATEK BgAffineSet: source entries are 18 bytes (4+4+2+2+2+2+2),
    // not 20. The second entry must be read at src+18. (Entries are
    // halfword-laid-out here: real tables are byte-packed and entries
    // past the first are never word-aligned, so 32-bit stores would
    // hit the bus align-down path instead of the intended bytes.)
    let mut regs = CpuRegisters::post_bios();
    let mut bus = test_bus();
    let src = 0x03000000;
    // Entry 0: identity (scale 1, no rotation, zero centers).
    for (off, v) in [(12, 0x100u16), (14, 0x100)] {
        bus.write16(src + off, v);
    }
    // Entry 1 at src+18: cx=0x1000, cy=0x2000, disp=(3,5), scale 1.
    let e1 = src + 18;
    bus.write16(e1, 0x1000);
    bus.write16(e1 + 2, 0x0000);
    bus.write16(e1 + 4, 0x2000);
    bus.write16(e1 + 6, 0x0000);
    bus.write16(e1 + 8, 3);
    bus.write16(e1 + 10, 5);
    bus.write16(e1 + 12, 0x100);
    bus.write16(e1 + 14, 0x100);
    bus.write16(e1 + 16, 0);
    regs.set_r(0, src);
    regs.set_r(1, 0x03000100);
    regs.set_r(2, 2);
    handle_swi(&mut regs, &mut bus, 0x0E);
    // Entry 0 outputs.
    assert_eq!(bus.read16(0x03000100), 0x100);
    assert_eq!(bus.read32(0x03000108), 0);
    // Entry 1 outputs: pa=0x100, start=(0x1000-3*0x100, 0x2000-5*0x100).
    assert_eq!(bus.read16(0x03000110), 0x100);
    assert_eq!(bus.read16(0x03000112), 0);
    assert_eq!(bus.read16(0x03000114), 0);
    assert_eq!(bus.read16(0x03000116), 0x100);
    assert_eq!(bus.read32(0x03000118), 0xD00);
    assert_eq!(bus.read32(0x0300011C), 0x1B00);
}

#[test]
fn protected_bios_latch_tracks_swi() {
    // jsmolka bios t001/t002 mechanism: protected reads return the
    // latched prefetch (repeat reads identical); an HLE SWI
    // re-latches 0xE3A02004. A cycling model fails the repeat check.
    let mut regs = CpuRegisters::post_bios();
    let mut bus = test_bus();
    bus.set_current_pc(0x08000000);
    assert_eq!(bus.read32(0), 0xE129F000);
    assert_eq!(bus.read32(0), 0xE129F000);
    regs.set_r(0, 0x1000);
    let _ = handle_swi(&mut regs, &mut bus, 0x08);
    assert_eq!(bus.read32(0), 0xE3A02004);
    assert_eq!(bus.read32(0), 0xE3A02004);
}

#[test]
fn soft_reset_clears_iwram_and_branches() {
    let mut regs = CpuRegisters::post_bios();
    let mut bus = test_bus();
    bus.write32(0x03007E00, 0xDEADBEEF);
    assert_eq!(handle_swi(&mut regs, &mut bus, 0), SwiResult::Branch(3));
    assert_eq!(regs.pc(), 0x08000000);
    // GBATEK SoftReset: System mode, SP_sys=0x03007F00 (SVC=0x7FE0,
    // IRQ=0x7FA0), ARM state, R0-R12 zeroed.
    assert_eq!(regs.cpsr_mode(), 0x1F);
    assert!(!regs.cpsr_t());
    assert_eq!(regs.sp(), 0x03007F00);
    assert_eq!(regs.r(0), 0);
    assert_eq!(bus.read32(0x03007E00), 0);
}

#[test]
fn soft_reset_can_boot_from_ewram() {
    let mut regs = CpuRegisters::post_bios();
    let mut bus = test_bus();
    bus.write8(0x03007FFA, 1);
    handle_swi(&mut regs, &mut bus, 0);
    assert_eq!(regs.pc(), 0x02000000);
}

#[test]
fn div_handles_minimum_without_panicking() {
    let mut regs = CpuRegisters::post_bios();
    let mut bus = test_bus();
    regs.set_r(0, i32::MIN as u32);
    regs.set_r(1, -1i32 as u32);
    handle_swi(&mut regs, &mut bus, 6);
    assert_eq!(regs.r(0), i32::MIN as u32);
    assert_eq!(regs.r(1), 0);
    assert_eq!(regs.r(3), 0x80000000);
}

#[test]
fn div_by_zero_uses_documented_result() {
    // r0 = sign(num), r1 = num, r3 = 1 (PeterLemon BIOSDIV).
    let mut regs = CpuRegisters::post_bios();
    let mut bus = test_bus();
    regs.set_r(0, -7i32 as u32);
    regs.set_r(1, 0);
    handle_swi(&mut regs, &mut bus, 6);
    assert_eq!(regs.r(0), u32::MAX);
    assert_eq!(regs.r(1), -7i32 as u32);
    assert_eq!(regs.r(3), 1);
    regs.set_r(0, 7);
    regs.set_r(1, 0);
    handle_swi(&mut regs, &mut bus, 6);
    assert_eq!(regs.r(0), 1);
    assert_eq!(regs.r(1), 7);
    assert_eq!(regs.r(3), 1);
}

#[test]
fn cpu_set_copies_and_fills() {
    let mut regs = CpuRegisters::post_bios();
    let mut bus = test_bus();
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
    let mut bus = test_bus();
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
    let mut bus = test_bus();
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

    assert_eq!(
        cycles,
        super::hle_operation::CPU_SET_SETUP_CYCLES
            + 2
            + super::hle_operation::CPU_SET_RETURN_CYCLES
    );
    assert_eq!(bus.read16(0x03000002), 0x1234);
}

#[test]
fn halt_waits_for_enabled_interrupt() {
    let mut regs = CpuRegisters::post_bios();
    let mut bus = test_bus();
    bus.write16(0x04000200, 1);
    handle_swi(&mut regs, &mut bus, 2);
    assert!(bus.is_halted());
    bus.request_interrupt(1);
    // Wake arrives once availability propagates (apply +1, avail +1).
    bus.tick();
    bus.tick();
    assert!(!bus.is_halted());
}

#[test]
fn intr_wait_discards_only_requested_flags() {
    let mut regs = CpuRegisters::post_bios();
    let mut bus = test_bus();
    bus.write16(0x04000200, 3);
    bus.request_interrupt(3);
    // Let IE/IF reach the effective registers before the SWI samples.
    bus.tick();
    bus.tick();
    regs.set_r(0, 1);
    regs.set_r(1, 1);
    handle_swi(&mut regs, &mut bus, 4);
    // The SWI's IF-ack applies 1 tick later (delayed pipeline).
    bus.tick();
    assert_eq!(bus.read16(0x04000202), 2);
    assert_eq!(bus.read16(0x03007FF8), 2);
    // GBATEK HALTCNT: halt wakes on ANY enabled interrupt, so the
    // still-raised HBlank flag wakes this VBlank-only wait at once
    // (on HW its ISR would run, then IntrWait re-halts). The old
    // narrow-wake model parked here instead; mgba-suite's layer-toggle
    // video tests prove the wide wake (mid-frame band missing).
    assert!(!bus.is_halted());
    // ...and with the awaited flags still clear, IRQ return re-halts.
    assert!(bus.rehalt_after_irq());
    assert!(bus.is_halted());
    // Awaited flags raised: the SWI exits, no re-halt.
    bus.request_interrupt(1);
    bus.tick();
    bus.tick();
    assert!(!bus.is_halted());
    assert!(!bus.rehalt_after_irq());
}

#[test]
fn arc_tan_fedcba98() {
    // SWI 0x09 ArcTan: R0=0xFEDCBA98 -> R0=0xFFFFE024 (HW CORDIC value,
    // pinned by the PeterLemon BIOSARCTAN HW reference image).
    // HLEの cycles は 0x6A だが、timer は start_delay=2 のため bus.tick() を cycles 回だけ
    // 回すと 0x68 になる。ROMでは `str r12,[r11]` の2サイクル overhead が加わり 0x6A で観測される。
    let mut regs = CpuRegisters::post_bios();
    let mut bus = test_bus();
    bus.write16(0x04000100, 0);
    bus.write16(0x04000102, 0x0080); // enable, prescaler 0
    regs.set_r(0, 0xFEDCBA98);
    let ret = handle_swi(&mut regs, &mut bus, 0x09);
    let cycles = match ret {
        SwiResult::Return(c) => c,
        _ => 0,
    };
    assert_eq!(regs.r(0), 0xFFFFE024, "ArcTan result mismatch");
    assert_eq!(cycles, 0x6A);
    for _ in 0..cycles {
        bus.tick();
    }
    // start_delay 2 により 2 少なくカウントされる
    assert_eq!(bus.read16(0x04000100), 0x0068);
    // ROMと同様に `str` の overhead 2 サイクルを加えると 0x6A になる
    for _ in 0..2 {
        bus.tick();
    }
    assert_eq!(bus.read16(0x04000100), 0x006A);
}

#[test]
fn div_e2() {
    // SWI 0x06 Div: TIMER0=0x00E2
    let mut regs = CpuRegisters::post_bios();
    let mut bus = test_bus();
    bus.write16(0x04000100, 0);
    bus.write16(0x04000102, 0x0080);
    regs.set_r(0, 0x12345678);
    regs.set_r(1, 0x1000);
    let ret = handle_swi(&mut regs, &mut bus, 0x06);
    let cycles = match ret {
        SwiResult::Return(c) => c,
        _ => 0,
    };
    assert_eq!(cycles, 0xE2);
    for _ in 0..cycles {
        bus.tick();
    }
    assert_eq!(bus.read16(0x04000100), 0x00E0);
    for _ in 0..2 {
        bus.tick();
    }
    assert_eq!(bus.read16(0x04000100), 0x00E2);
}

#[test]
fn div_arm_e5() {
    // SWI 0x07 DivArm: TIMER0=0x00E5 (Divより3cyc増)
    let mut regs = CpuRegisters::post_bios();
    let mut bus = test_bus();
    bus.write16(0x04000100, 0);
    bus.write16(0x04000102, 0x0080);
    regs.set_r(0, 0x1000);
    regs.set_r(1, 0x12345678);
    let ret = handle_swi(&mut regs, &mut bus, 0x07);
    let cycles = match ret {
        SwiResult::Return(c) => c,
        _ => 0,
    };
    assert_eq!(cycles, 0xE5);
    for _ in 0..cycles {
        bus.tick();
    }
    assert_eq!(bus.read16(0x04000100), 0x00E3);
    for _ in 0..2 {
        bus.tick();
    }
    assert_eq!(bus.read16(0x04000100), 0x00E5);
}

#[test]
fn sqrt_fedcba98() {
    // SWI 0x08 Sqrt: R0=0xFEDCBA98 -> R0=0xFF6E, TIMER0=0x0249
    let mut regs = CpuRegisters::post_bios();
    let mut bus = test_bus();
    bus.write16(0x04000100, 0);
    bus.write16(0x04000102, 0x0080);
    regs.set_r(0, 0xFEDCBA98);
    let ret = handle_swi(&mut regs, &mut bus, 0x08);
    let cycles = match ret {
        SwiResult::Return(c) => c,
        _ => 0,
    };
    assert_eq!(regs.r(0), 0xFF6E, "Sqrt result mismatch");
    assert_eq!(cycles, 0x249);
    for _ in 0..cycles {
        bus.tick();
    }
    assert_eq!(bus.read16(0x04000100), 0x0247);
    for _ in 0..2 {
        bus.tick();
    }
    assert_eq!(bus.read16(0x04000100), 0x0249);
}

#[test]
fn arc_tan2_fedcba98_12345678() {
    let mut regs = CpuRegisters::post_bios();
    let mut bus = test_bus();
    bus.write16(0x04000100, 0);
    bus.write16(0x04000102, 0x0080);
    regs.set_r(0, 0xFEDCBA98);
    regs.set_r(1, 0x12345678);
    let ret = handle_swi(&mut regs, &mut bus, 0x0A);
    let cycles = match ret {
        SwiResult::Return(c) => c,
        _ => 0,
    };
    assert_eq!(regs.r(0), 0x00003FFF, "ArcTan2 result mismatch");
    assert_eq!(cycles, 0xC8);
    for _ in 0..cycles {
        bus.tick();
    }
    assert_eq!(bus.read16(0x04000100), 0x00C6);
    for _ in 0..2 {
        bus.tick();
    }
    assert_eq!(bus.read16(0x04000100), 0x00C8);
}

#[test]
fn arctan2_fourth_quadrant_shallow_angles() {
    // SWI 0x0A ArcTan2, fourth quadrant (x > 0, y < 0): shallow angles
    // (|x| >= |y|) must use the reciprocal so the polynomial stays in
    // its |ratio| <= 1 domain. Feeding x/y directly overflows the
    // fixed-point square and returns angles tens of degrees off, which
    // visibly breaks Mario Kart Super Circuit kart/camera angles
    // (karts vanish into a spinning camera ~1s after the race start).
    // Values below match f64 atan2 mapped to GBA units within 1 LSB.
    for ((x, y), expected) in [
        ((789i32, -131i32), 0xF94C),
        ((63, -52), 0xE3E2),
        ((100, -100), 0xE000),
        ((100, -101), 0xDFCC),
        ((50, -100), 0xD2E4),
        ((1, -1), 0xE000),
        ((1000, -1), 0xFFF5),
        ((760, -211), 0xF4F7),
        ((63, -211), 0xCBD2),
    ] {
        let (v, _) = bios_arctan2_full(x, y);
        assert_eq!(v, expected, "ArcTan2({x},{y}) mismatch");
    }
}
