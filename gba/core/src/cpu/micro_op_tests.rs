//! Differential tests: micro-op engine vs the legacy instruction stepper.
//! Lives in a sibling of `micro_op` (not a child) so the file dependency
//! stays one-way: `cpu -> micro_op`, `cpu -> micro_op_tests`,
//! `micro_op_tests -> {micro_op, cpu}`, with nothing pointing back at
//! `micro_op_tests`.
use super::micro_op::{
    MicroOp, expand_arm, expand_arm_dp_reg, expand_arm_mul, expand_arm_single, expand_thumb,
    expand_thumb_alu_rest, step_op,
};
use crate::cpu::GbaCpu;
use crate::cpu_pipeline::fill_pipeline;
use crate::cpu_registers::CpuRegisters;
use crate::memory::GbaMemoryBus;

/// Run `steps` instructions under both engines from identical state.
/// Returns (legacy_total, micro_total, regs_equal, followup_equal).
/// `code_base` locates the corpus (IWRAM for writable code, ROM with
/// `cart` for GamePak-code paths); `mem_init`/`reg_init` preset memory
/// (addr, width, value) and registers before the pipeline fill.
#[allow(clippy::too_many_arguments)]
fn differential(
    code: &[u32],
    thumb: bool,
    waitcnt: u16,
    steps: usize,
    followup: u32,
    code_base: u32,
    cart: Option<Vec<u8>>,
    mem_init: &[(u32, u8, u32)],
    reg_init: &[(usize, u32)],
) -> (u32, u32, bool, bool) {
    let (mut a_cpu, mut a_bus, _) = setup_corpus(
        code,
        thumb,
        waitcnt,
        code_base,
        cart.clone(),
        mem_init,
        reg_init,
    );
    let (mut b_cpu, mut b_bus, mut b_pipe) =
        setup_corpus(code, thumb, waitcnt, code_base, cart, mem_init, reg_init);
    // Twin-bus check: both setups must agree before stepping.
    assert_eq!(a_cpu.pipeline, b_pipe);
    let (mut ta, mut tb) = (0u32, 0u32);
    let mut b_queue = std::collections::VecDeque::new();
    for _ in 0..steps {
        let (da, db) = step_pair(
            &mut a_cpu,
            &mut a_bus,
            &mut b_cpu,
            &mut b_bus,
            &mut b_pipe,
            &mut b_queue,
            thumb,
        );
        ta += da;
        tb += db;
    }
    let regs_equal = (0..16).all(|r| a_cpu.regs.r(r) == b_cpu.regs.r(r))
        && a_cpu.regs.cpsr() == b_cpu.regs.cpsr();
    let same_followup = followup_check(
        &mut a_cpu, &mut a_bus, &mut b_cpu, &mut b_bus, &b_pipe, followup, thumb,
    );
    (ta, tb, regs_equal, same_followup)
}

/// Build one twin (CPU + bus + shadow pipeline) for the differential.
/// `code_base` locates the corpus (IWRAM for writable code, ROM with
/// `cart` for GamePak-code paths); `mem_init`/`reg_init` preset memory
/// (addr, width, value) and registers before the pipeline fill.
#[allow(clippy::too_many_arguments)]
fn setup_corpus(
    code: &[u32],
    thumb: bool,
    waitcnt: u16,
    code_base: u32,
    cart: Option<Vec<u8>>,
    mem_init: &[(u32, u8, u32)],
    reg_init: &[(usize, u32)],
) -> (GbaCpu, GbaMemoryBus, [u32; 2]) {
    let mut cpu = GbaCpu::post_bios();
    let mut bus = GbaMemoryBus::new();
    bus.write16(0x04000204, waitcnt);
    load_corpus(&mut bus, code, thumb, code_base, cart);
    for (addr, width, val) in mem_init {
        match width {
            4 => bus.write32(*addr, *val),
            2 => bus.write16(*addr, (*val & 0xFFFF) as u16),
            _ => bus.write8(*addr, (*val & 0xFF) as u8),
        }
    }
    for (r, v) in reg_init {
        cpu.regs.set_r(*r, *v);
    }
    // Stack for the follow-up load.
    bus.write16(0x0300_7F00, 0x1234);
    cpu.regs.set_r(13, 0x0300_7F00);
    if thumb {
        cpu.regs.set_cpsr(cpu.regs.cpsr() | (1 << 5));
    }
    cpu.regs.set_pc(code_base);
    fill_pipeline(&mut cpu.regs, &mut bus, &mut cpu.pipeline);
    bus.take_access_wait_cycles();
    // Shadow pipeline for the interpreter (same fetches).
    let mut shadow = [0u32; 2];
    // Reproduce the fill through the same bus calls: reset and
    // refill identically by re-driving fill on a twin bus is
    // overkill; instead mirror the two fetch values.
    shadow[0] = cpu.pipeline[0];
    shadow[1] = cpu.pipeline[1];
    (cpu, bus, shadow)
}

/// Lay the corpus down (IWRAM at `code_base`, or ROM image at +0x100).
fn load_corpus(
    bus: &mut GbaMemoryBus,
    code: &[u32],
    thumb: bool,
    code_base: u32,
    cart: Option<Vec<u8>>,
) {
    let stride = if thumb { 2 } else { 4 };
    if let Some(mut rom) = cart {
        for (i, w) in code.iter().enumerate() {
            let o = 0x100 + i * stride as usize;
            if thumb {
                rom[o] = (w & 0xFF) as u8;
                rom[o + 1] = (w >> 8) as u8;
            } else {
                rom[o..o + 4].copy_from_slice(&w.to_le_bytes());
            }
        }
        bus.set_cartridge(crate::cartridge::Cartridge::new(rom).unwrap());
    } else {
        for (i, w) in code.iter().enumerate() {
            let addr = code_base + (i as u32) * stride as u32;
            if thumb {
                bus.write16(addr, (w & 0xFFFF) as u16);
            } else {
                bus.write32(addr, *w);
            }
        }
    }
}

/// One differential iteration: a legacy oracle step plus a full
/// micro-op instruction drain on the twin. Returns (legacy, micro).
fn step_pair(
    a_cpu: &mut GbaCpu,
    a_bus: &mut GbaMemoryBus,
    b_cpu: &mut GbaCpu,
    b_bus: &mut GbaMemoryBus,
    b_pipe: &mut [u32; 2],
    b_queue: &mut std::collections::VecDeque<MicroOp>,
    thumb: bool,
) -> (u32, u32) {
    // Legacy oracle: step_legacy bypasses the micro-op wiring so
    // the harness stays a true differential even once covered
    // classes route through step_op in production.
    let legacy = a_cpu.step_legacy(a_bus);
    // Micro-op engine on the twin: drain one full instruction
    // (the queue may span several step_op calls), flooring once
    // at retire exactly like the legacy step.
    let mut acc = 0i64;
    loop {
        acc += step_op(&mut b_cpu.regs, b_bus, b_pipe, b_queue, thumb)
            .expect("corpus must be covered");
        if b_queue.is_empty() {
            break;
        }
    }
    (legacy, acc.max(1) as u32)
}

/// Follow-up load through the legacy engine on both buses: detects
/// fetch-stream/erase-state divergence.
fn followup_check(
    a_cpu: &mut GbaCpu,
    a_bus: &mut GbaMemoryBus,
    b_cpu: &mut GbaCpu,
    b_bus: &mut GbaMemoryBus,
    b_pipe: &[u32; 2],
    followup: u32,
    thumb: bool,
) -> bool {
    // Point both at the follow-up instruction.
    b_cpu.pipeline = *b_pipe;
    a_cpu.regs.set_pc(0x0300_0000);
    b_cpu.regs.set_pc(0x0300_0000);
    fill_pipeline(&mut a_cpu.regs, a_bus, &mut a_cpu.pipeline);
    fill_pipeline(&mut b_cpu.regs, b_bus, &mut b_cpu.pipeline);
    a_bus.take_access_wait_cycles();
    b_bus.take_access_wait_cycles();
    let fa = run_one_legacy(a_cpu, a_bus, followup, thumb);
    let fb = run_one_legacy(b_cpu, b_bus, followup, thumb);
    fa == fb
}

/// Execute one arbitrary instruction via the legacy engine (bypasses
/// the micro-op wiring like the differential oracle).
fn run_one_legacy(cpu: &mut GbaCpu, bus: &mut GbaMemoryBus, word: u32, thumb: bool) -> u32 {
    let base = 0x0300_0100u32;
    if thumb {
        bus.write16(base, (word & 0xFFFF) as u16);
        bus.write16(base + 2, (word >> 16) as u16);
        cpu.regs.set_cpsr(cpu.regs.cpsr() | (1 << 5));
    } else {
        bus.write32(base, word);
    }
    cpu.regs.set_pc(base);
    fill_pipeline(&mut cpu.regs, bus, &mut cpu.pipeline);
    bus.take_access_wait_cycles();
    cpu.step_legacy(bus)
}

// Corpus: ALU-imm + B, ARM and Thumb. Encodings hand-checked against
// the existing handler unit tests.
const ARM_CORPUS: [u32; 6] = [
    0xE3A0_0001, // mov r0, #1
    0xE281_1002, // add r1, r1, #2
    0xE252_2003, // subs r2, r2, #3
    0xE354_0004, // cmp r4, #4
    0xEA00_0001, // b +12 (skips one)
    0xE3A0_3005, // mov r3, #5
];
const THUMB_CORPUS: [u32; 6] = [
    0x2001, // mov r0, #1
    0x3102, // add r1, #2
    0x3A03, // sub r2, #3
    0x2C04, // cmp r4, #4
    0xE001, // b +4 (skips one)
    0x2305, // mov r3, #5
];

#[test]
fn micro_op_matches_legacy_arm() {
    for waitcnt in [0x0000u16, 0x0010, 0x4000, 0x4014] {
        let (ta, tb, regs, follow) = differential(
            &ARM_CORPUS,
            false,
            waitcnt,
            5,
            0xE1DD_20B0,
            0x0300_0000,
            None,
            &[],
            &[],
        );
        assert_eq!((ta, tb), (ta, ta), "waitcnt={waitcnt:#06x}");
        assert!(regs, "waitcnt={waitcnt:#06x}");
        assert!(follow, "waitcnt={waitcnt:#06x}");
    }
}

#[test]
fn micro_op_matches_legacy_thumb() {
    for waitcnt in [0x0000u16, 0x0010, 0x4000, 0x4014] {
        let (ta, tb, regs, follow) = differential(
            &THUMB_CORPUS,
            true,
            waitcnt,
            5,
            0x886A,
            0x0300_0000,
            None,
            &[],
            &[],
        );
        assert_eq!((ta, tb), (ta, ta), "waitcnt={waitcnt:#06x}");
        assert!(regs, "waitcnt={waitcnt:#06x}");
        assert!(follow, "waitcnt={waitcnt:#06x}");
    }
}

#[test]
fn conditional_branch_loops_match_legacy() {
    let arm = [
        0xE3A0_0000, // mov r0, #0
        0xE280_0001, // add r0, r0, #1
        0xE350_0003, // cmp r0, #3
        0x1AFF_FFFC, // bne back to add
        0xE3A0_1007, // mov r1, #7
    ];
    let thumb = [
        0x2000, // mov r0, #0
        0x3001, // add r0, #1
        0x2803, // cmp r0, #3
        0xD1FC, // bne back to add
        0x2107, // mov r1, #7
    ];
    for waitcnt in [0x0000u16, 0x4010, 0x4014] {
        let (ta, tb, regs, follow) = differential(
            &arm,
            false,
            waitcnt,
            11,
            0xE3A0_2009,
            0x0300_0000,
            None,
            &[],
            &[],
        );
        assert_eq!((ta, tb), (ta, ta), "ARM waitcnt={waitcnt:#06x}");
        assert!(regs && follow, "ARM waitcnt={waitcnt:#06x}");

        let (ta, tb, regs, follow) = differential(
            &thumb,
            true,
            waitcnt,
            11,
            0x2209,
            0x0300_0000,
            None,
            &[],
            &[],
        );
        assert_eq!((ta, tb), (ta, ta), "Thumb waitcnt={waitcnt:#06x}");
        assert!(regs && follow, "Thumb waitcnt={waitcnt:#06x}");
    }
}

#[test]
fn conditional_alu_and_memory_match_legacy() {
    let code = [
        0xE3A0_0000, // mov r0, #0
        0xE350_0000, // cmp r0, #0 (Z=1)
        0x0281_1001, // addeq r1, r1, #1
        0x1282_2001, // addne r2, r2, #1 (skipped)
        0x0583_1000, // streq r1, [r3]
        0x1593_4000, // ldrne r4, [r3] (skipped)
        0xE593_5000, // ldr r5, [r3]
    ];
    for waitcnt in [0x0000u16, 0x4010, 0x4014] {
        let (ta, tb, regs, follow) = differential(
            &code,
            false,
            waitcnt,
            code.len(),
            0xE3A0_6009,
            0x0300_0000,
            None,
            &[],
            &[(1, 4), (2, 8), (3, 0x0200_0100)],
        );
        assert_eq!((ta, tb), (ta, ta), "waitcnt={waitcnt:#06x}");
        assert!(regs && follow, "waitcnt={waitcnt:#06x}");
    }
}

// Slice 2 corpus: single load/store (word + halfword, ARM + Thumb).
// Pointers preset in Rust; r0 carries the store value.
const ARM_LS_CORPUS: [u32; 8] = [
    0xE3A0_1402, // mov r1, #0x02000000 (EWRAM ptr; rot, S=0)
    0xE3A0_1403, // mov r2, #0x03000000 (IWRAM ptr; rot, S=0)
    0xE581_0004, // str r0, [r1, #4]
    0xE591_3004, // ldr r3, [r1, #4]
    0xE1C2_00B8, // strh r0, [r2, #8]
    0xE1D2_40B8, // ldrh r4, [r2, #8]
    0xE591_5008, // ldr r5, [r1, #8]
    0xE3A0_0007, // mov r7, #7
];
const ARM_INDEXED_LS_CORPUS: [u32; 8] = [
    0xE4C1_0001, // strb r0, [r1], #1
    0xE551_2001, // ldrb r2, [r1, #-1]
    0xE5E1_0002, // strb r0, [r1, #2]!
    0xE411_3004, // ldr r3, [r1], #-4
    0xE0C1_00B2, // strh r0, [r1], #2
    0xE171_40B2, // ldrh r4, [r1, #-2]!
    0xE591_5000, // ldr r5, [r1]
    0xE3A0_6007, // mov r6, #7
];
const THUMB_LS_CORPUS: [u32; 7] = [
    0x6088, // str r0, [r1, #8]
    0x688B, // ldr r3, [r1, #8]
    0x8090, // strh r0, [r2, #4]
    0x8894, // ldrh r4, [r2, #4]
    0x9010, // str r0, [sp, #0x40]
    0x9D10, // ldr r5, [sp, #0x40]
    0x2707, // mov r7, #7
];
const THUMB_REG_LS_CORPUS: [u32; 8] = [
    0x5088, // str r0, [r1, r2]
    0x588B, // ldr r3, [r1, r2]
    0x5488, // strb r0, [r1, r2]
    0x5C8C, // ldrb r4, [r1, r2]
    0x5288, // strh r0, [r1, r2]
    0x5A8D, // ldrh r5, [r1, r2]
    0x568E, // ldrsb r6, [r1, r2]
    0x5E8F, // ldrsh r7, [r1, r2]
];
// ROM-code corpus: same shapes from GamePak (prefetch/erase paths)
// plus a ROM-data read. Code at 0x08000100, data word at 0x08000200.
const ARM_ROM_CORPUS: [u32; 9] = [
    0xE3A0_1402, // mov r1, #0x02000000
    0xE3A0_1403, // mov r2, #0x03000000
    0xE3A0_1408, // mov r4, #0x08000000 (ROM ptr; rot, S=0)
    0xE581_0004, // str r0, [r1, #4]
    0xE591_3004, // ldr r3, [r1, #4]
    0xE1C2_00B8, // strh r0, [r2, #8]
    0xE1D2_50B8, // ldrh r5, [r2, #8]
    0xE594_6000, // ldr r6, [r4, #0] (ROM data)
    0xE3A0_0007, // mov r7, #7
];

fn rom_cart() -> Vec<u8> {
    const SUITE: &[u8] = include_bytes!("../../../../roms/gba/mgba-suite/suite.gba");
    let mut rom = vec![0u8; 0x10000];
    rom[..0xC0].copy_from_slice(&SUITE[..0xC0]);
    rom[0x200..0x204].copy_from_slice(&0xCAFE_BABEu32.to_le_bytes());
    rom
}

#[test]
fn micro_op_load_store_matches_legacy() {
    for waitcnt in [0x0000u16, 0x0010, 0x4000, 0x4010, 0x4014] {
        let (ta, tb, regs, follow) = differential(
            &ARM_LS_CORPUS,
            false,
            waitcnt,
            8,
            0xE1DD_20B0,
            0x0300_0000,
            None,
            &[(0x0200_0008, 4, 0xAABB_CCDD)],
            &[(0, 0x1234_5678)],
        );
        assert_eq!((ta, tb), (ta, ta), "arm waitcnt={waitcnt:#06x}");
        assert!(regs, "arm waitcnt={waitcnt:#06x}");
        assert!(follow, "arm waitcnt={waitcnt:#06x}");
        let (ta, tb, regs, follow) = differential(
            &ARM_INDEXED_LS_CORPUS,
            false,
            waitcnt,
            ARM_INDEXED_LS_CORPUS.len(),
            0xE1DD_20B0,
            0x0300_0000,
            None,
            &[],
            &[(0, 0x1234_80FF), (1, 0x0300_0200)],
        );
        assert_eq!((ta, tb), (ta, ta), "arm-indexed waitcnt={waitcnt:#06x}");
        assert!(regs, "arm-indexed waitcnt={waitcnt:#06x}");
        assert!(follow, "arm-indexed waitcnt={waitcnt:#06x}");
        let (ta, tb, regs, follow) = differential(
            &THUMB_LS_CORPUS,
            true,
            waitcnt,
            7,
            0x886A,
            0x0300_0000,
            None,
            &[(0x0200_0008, 4, 0xAABB_CCDD)],
            &[(0, 0x1234_5678), (1, 0x0200_0000), (2, 0x0300_0300)],
        );
        assert_eq!((ta, tb), (ta, ta), "thumb waitcnt={waitcnt:#06x}");
        assert!(regs, "thumb waitcnt={waitcnt:#06x}");
        assert!(follow, "thumb waitcnt={waitcnt:#06x}");

        let (ta, tb, regs, follow) = differential(
            &THUMB_REG_LS_CORPUS,
            true,
            waitcnt,
            THUMB_REG_LS_CORPUS.len(),
            0x886A,
            0x0300_0000,
            None,
            &[],
            &[(0, 0x0000_80FF), (1, 0x0300_0100), (2, 4)],
        );
        assert_eq!((ta, tb), (ta, ta), "thumb-reg waitcnt={waitcnt:#06x}");
        assert!(regs, "thumb-reg waitcnt={waitcnt:#06x}");
        assert!(follow, "thumb-reg waitcnt={waitcnt:#06x}");
    }
}

// Slice 3 corpus: Thumb PUSH/POP (non-empty; empty forms stay
// legacy). The harness presets SP=0x03007F00; pushes spill below
// and pops reload them (LIFO-balanced across the corpus).
const THUMB_PP_CORPUS: [u32; 5] = [
    0xB40F, // push {r0-r3}
    0xBCF0, // pop {r4-r7}
    0xB550, // push {r4,r6,lr}
    0xBC0B, // pop {r0,r1,r3}
    0x2707, // mov r7, #7
];

#[test]
fn micro_op_push_pop_matches_legacy() {
    let regs = [
        (0, 0x1111_1111),
        (1, 0x2222_2222),
        (2, 0x3333_3333),
        (3, 0x4444_4444),
        (4, 0x5555_5555),
        (6, 0x6666_6666),
        (14, 0xDEAD_BEEF),
    ];
    for waitcnt in [0x0000u16, 0x0010, 0x4000, 0x4010, 0x4014] {
        let (ta, tb, regs_equal, follow) = differential(
            &THUMB_PP_CORPUS,
            true,
            waitcnt,
            THUMB_PP_CORPUS.len(),
            0x886A,
            0x0300_0000,
            None,
            &[],
            &regs,
        );
        assert_eq!((ta, tb), (ta, ta), "iwram waitcnt={waitcnt:#06x}");
        assert!(regs_equal, "iwram waitcnt={waitcnt:#06x}");
        assert!(follow, "iwram waitcnt={waitcnt:#06x}");
    }
    // ROM-code variant: words still hit IWRAM, but the block batch
    // erase path is the ROM-code one (whole-word total accounting).
    for waitcnt in [0x0000u16, 0x4010, 0x4014] {
        let (ta, tb, regs_equal, follow) = differential(
            &THUMB_PP_CORPUS,
            true,
            waitcnt,
            THUMB_PP_CORPUS.len(),
            0x886A,
            0x0800_0100,
            Some(rom_cart()),
            &[],
            &regs,
        );
        assert_eq!((ta, tb), (ta, ta), "rom waitcnt={waitcnt:#06x}");
        assert!(regs_equal, "rom waitcnt={waitcnt:#06x}");
        assert!(follow, "rom waitcnt={waitcnt:#06x}");
    }
}

#[test]
fn empty_push_pop_stays_legacy() {
    fn regs_with_sp(sp: u32) -> CpuRegisters {
        let mut regs = CpuRegisters::post_bios();
        regs.set_sp(sp);
        regs
    }
    let regs = regs_with_sp(0x0300_7F00);
    assert!(expand_thumb(0xB400, &regs).is_none());
    assert!(expand_thumb(0xBC00, &regs).is_none());
    let ops = expand_thumb(0xB40F, &regs).expect("non-empty pushes expand");
    // BlockStart + 4 words + BlockEnd + 1 trailing = 7.
    assert_eq!(ops.len(), 7);
}

/// PUSH/POP memory effects under both engines from identical state:
/// stack words as well as registers and totals must agree.
#[test]
fn push_pop_memory_matches_legacy() {
    fn setup() -> (GbaCpu, GbaMemoryBus) {
        let mut cpu = GbaCpu::post_bios();
        let mut bus = GbaMemoryBus::new();
        let code = THUMB_PP_CORPUS;
        for (i, w) in code.iter().enumerate() {
            bus.write16(0x0300_0000 + (i as u32) * 2, (*w & 0xFFFF) as u16);
        }
        for (r, v) in [
            (0, 0x1111_1111),
            (1, 0x2222_2222),
            (2, 0x3333_3333),
            (3, 0x4444_4444),
        ] {
            cpu.regs.set_r(r, v);
        }
        cpu.regs.set_r(4, 0x5555_5555);
        cpu.regs.set_r(6, 0x6666_6666);
        cpu.regs.set_lr(0xDEAD_BEEF);
        cpu.regs.set_cpsr(cpu.regs.cpsr() | (1 << 5));
        cpu.regs.set_pc(0x0300_0000);
        fill_pipeline(&mut cpu.regs, &mut bus, &mut cpu.pipeline);
        bus.take_access_wait_cycles();
        (cpu, bus)
    }
    fn drain_micro(cpu: &mut GbaCpu, bus: &mut GbaMemoryBus, steps: usize) -> u32 {
        let mut total = 0u32;
        let mut queue = std::collections::VecDeque::new();
        for _ in 0..steps {
            let mut acc = 0i64;
            loop {
                acc += step_op(&mut cpu.regs, bus, &mut cpu.pipeline, &mut queue, true)
                    .expect("corpus must be covered");
                if queue.is_empty() {
                    break;
                }
            }
            total += acc.max(1) as u32;
        }
        total
    }
    let (mut a_cpu, mut a_bus) = setup();
    let (mut b_cpu, mut b_bus) = setup();
    let mut ta = 0u32;
    for _ in 0..THUMB_PP_CORPUS.len() {
        ta += a_cpu.step_legacy(&mut a_bus);
    }
    let tb = drain_micro(&mut b_cpu, &mut b_bus, THUMB_PP_CORPUS.len());
    assert_eq!((ta, tb), (ta, ta));
    for r in 0..16 {
        assert_eq!(a_cpu.regs.r(r), b_cpu.regs.r(r), "r{r}");
    }
    assert_eq!(a_cpu.regs.cpsr(), b_cpu.regs.cpsr());
    for addr in (0x0300_7EE0..0x0300_7F00).step_by(4) {
        assert_eq!(a_bus.read32(addr), b_bus.read32(addr), "stack {addr:#010X}");
    }
}

/// POP {..,PC} through micro-ops: PC lands masked, SP advances past
/// both words, and the retire flushes the pipeline at the target.
#[test]
fn pop_pc_matches_legacy_and_flushes() {
    fn setup() -> (GbaCpu, GbaMemoryBus) {
        let mut cpu = GbaCpu::post_bios();
        let mut bus = GbaMemoryBus::new();
        bus.write16(0x0300_0000, 0xBD02); // pop {r1, pc}
        bus.write32(0x0300_7EF8, 0xCAFE_BABE);
        bus.write32(0x0300_7EFC, 0x0300_0041); // bit0 set: Thumb stays
        bus.write16(0x0300_0040, 0x2707); // mov r7, #7 (target)
        cpu.regs.set_cpsr(cpu.regs.cpsr() | (1 << 5));
        cpu.regs.set_pc(0x0300_0000);
        cpu.regs.set_sp(0x0300_7EF8);
        fill_pipeline(&mut cpu.regs, &mut bus, &mut cpu.pipeline);
        bus.take_access_wait_cycles();
        (cpu, bus)
    }
    let (mut a_cpu, mut a_bus) = setup();
    let ta = a_cpu.step_legacy(&mut a_bus);
    let (mut b_cpu, mut b_bus) = setup();
    let mut queue = std::collections::VecDeque::new();
    let mut acc = 0i64;
    loop {
        acc += step_op(
            &mut b_cpu.regs,
            &mut b_bus,
            &mut b_cpu.pipeline,
            &mut queue,
            true,
        )
        .expect("pop-pc must be covered");
        if queue.is_empty() {
            break;
        }
    }
    let tb = acc.max(1) as u32;
    assert_eq!((ta, tb), (ta, ta));
    assert_eq!(a_cpu.regs.r(1), 0xCAFE_BABE);
    assert_eq!(b_cpu.regs.r(1), 0xCAFE_BABE);
    assert_eq!(b_cpu.regs.pc(), 0x0300_0044); // target + Thumb pipeline lead
    assert_eq!(b_cpu.regs.pc(), a_cpu.regs.pc());
    assert_eq!(b_cpu.regs.sp(), 0x0300_7F00);
    assert_eq!(b_cpu.regs.sp(), a_cpu.regs.sp());
}

#[test]
fn push_pop_tick_parity() {
    let regs = [
        (0, 0x1111_1111),
        (1, 0x2222_2222),
        (2, 0x3333_3333),
        (3, 0x4444_4444),
        (4, 0x5555_5555),
        (6, 0x6666_6666),
        (14, 0xDEAD_BEEF),
    ];
    let (at, bt, av, bv) = tick_parity(
        &THUMB_PP_CORPUS,
        true,
        0x0000,
        0x0300_0000,
        &regs,
        THUMB_PP_CORPUS.len(),
    );
    assert_eq!((at, bt), (at, at), "ticks diverge");
    assert_eq!((av, bv), (av, av), "timer span diverges");
}

// Slice 4 corpus: Thumb LDMIA/STMIA (non-empty; empty forms stay
// legacy). Bases preset in Rust; r1/r2 start at A, r3 at A+16.
// The corpus exercises the STM stored-base quirk (r1 in its own
// list) and the LDM base-in-list no-writeback rule (r3).
const THUMB_MULT_CORPUS: [u32; 5] = [
    0xC10F, // stmia r1!, {r0-r3}
    0xCBF0, // ldmia r2!, {r4-r7}
    0xC205, // stmia r2!, {r2,r0} (stored-base quirk)
    0xCB18, // ldmia r3, {r3,r4} (base in list: no writeback)
    0x2707, // mov r7, #7
];
const THUMB_MULT_REGS: [(usize, u32); 7] = [
    (0, 0x1111_1111),
    (1, 0x0300_0100),
    (2, 0x0300_0100),
    (3, 0x0300_0110),
    (4, 0x4444_4444),
    (5, 0x5555_5555),
    (6, 0x6666_6666),
];

#[test]
fn micro_op_ldm_stm_matches_legacy() {
    for waitcnt in [0x0000u16, 0x0010, 0x4000, 0x4010, 0x4014] {
        let (ta, tb, regs_equal, follow) = differential(
            &THUMB_MULT_CORPUS,
            true,
            waitcnt,
            THUMB_MULT_CORPUS.len(),
            0x886A,
            0x0300_0000,
            None,
            &[],
            &THUMB_MULT_REGS,
        );
        assert_eq!((ta, tb), (ta, ta), "iwram waitcnt={waitcnt:#06x}");
        assert!(regs_equal, "iwram waitcnt={waitcnt:#06x}");
        assert!(follow, "iwram waitcnt={waitcnt:#06x}");
    }
    // ROM-code variant: data still hits IWRAM, batch erase is the
    // ROM-code whole-word path.
    for waitcnt in [0x0000u16, 0x4010, 0x4014] {
        let (ta, tb, regs_equal, follow) = differential(
            &THUMB_MULT_CORPUS,
            true,
            waitcnt,
            THUMB_MULT_CORPUS.len(),
            0x886A,
            0x0800_0100,
            Some(rom_cart()),
            &[],
            &THUMB_MULT_REGS,
        );
        assert_eq!((ta, tb), (ta, ta), "rom waitcnt={waitcnt:#06x}");
        assert!(regs_equal, "rom waitcnt={waitcnt:#06x}");
        assert!(follow, "rom waitcnt={waitcnt:#06x}");
    }
}

#[test]
fn empty_ldm_stm_stays_legacy() {
    let mut regs = CpuRegisters::post_bios();
    regs.set_r(1, 0x0300_0100);
    assert!(expand_thumb(0xC000, &regs).is_none());
    assert!(expand_thumb(0xC800, &regs).is_none());
    let ops = expand_thumb(0xC10F, &regs).expect("non-empty stmia expands");
    // BlockStart + 4 words + BlockEnd + 1 trailing = 7.
    assert_eq!(ops.len(), 7);
}

/// LDM/STM memory effects under both engines: block words as well
/// as registers and totals must agree.
#[test]
fn ldm_stm_memory_matches_legacy() {
    fn setup() -> (GbaCpu, GbaMemoryBus) {
        let mut cpu = GbaCpu::post_bios();
        let mut bus = GbaMemoryBus::new();
        for (i, w) in THUMB_MULT_CORPUS.iter().enumerate() {
            bus.write16(0x0300_0000 + (i as u32) * 2, (*w & 0xFFFF) as u16);
        }
        for (r, v) in THUMB_MULT_REGS {
            cpu.regs.set_r(r, v);
        }
        cpu.regs.set_cpsr(cpu.regs.cpsr() | (1 << 5));
        cpu.regs.set_pc(0x0300_0000);
        fill_pipeline(&mut cpu.regs, &mut bus, &mut cpu.pipeline);
        bus.take_access_wait_cycles();
        (cpu, bus)
    }
    fn drain_micro(cpu: &mut GbaCpu, bus: &mut GbaMemoryBus, steps: usize) -> u32 {
        let mut total = 0u32;
        let mut queue = std::collections::VecDeque::new();
        for _ in 0..steps {
            let mut acc = 0i64;
            loop {
                acc += step_op(&mut cpu.regs, bus, &mut cpu.pipeline, &mut queue, true)
                    .expect("corpus must be covered");
                if queue.is_empty() {
                    break;
                }
            }
            total += acc.max(1) as u32;
        }
        total
    }
    let (mut a_cpu, mut a_bus) = setup();
    let (mut b_cpu, mut b_bus) = setup();
    let mut ta = 0u32;
    for _ in 0..THUMB_MULT_CORPUS.len() {
        ta += a_cpu.step_legacy(&mut a_bus);
    }
    let tb = drain_micro(&mut b_cpu, &mut b_bus, THUMB_MULT_CORPUS.len());
    assert_eq!((ta, tb), (ta, ta));
    for r in 0..16 {
        assert_eq!(a_cpu.regs.r(r), b_cpu.regs.r(r), "r{r}");
    }
    assert_eq!(a_cpu.regs.cpsr(), b_cpu.regs.cpsr());
    for addr in (0x0300_0100..0x0300_0130).step_by(4) {
        assert_eq!(a_bus.read32(addr), b_bus.read32(addr), "block {addr:#010X}");
    }
}

#[test]
fn ldm_stm_tick_parity() {
    let (at, bt, av, bv) = tick_parity(
        &THUMB_MULT_CORPUS,
        true,
        0x0000,
        0x0300_0000,
        &THUMB_MULT_REGS,
        THUMB_MULT_CORPUS.len(),
    );
    assert_eq!((at, bt), (at, at), "ticks diverge");
    assert_eq!((av, bv), (av, av), "timer span diverges");
}

// Slice 5a corpus: ARM LDM/STM (non-empty, S=0; empty forms stay
// legacy; S-bit forms are covered by the slice-14 corpus below).
// This corpus exercises IA/DB modes, the STM stored-base quirk
// (r0 in its own list) and the LDM base-in-list no-writeback rule.
const ARM_BLOCK_CORPUS: [u32; 6] = [
    0xE8A0_0006, // stmia r0!, {r1,r2}
    0xE8B5_0018, // ldmia r5!, {r3,r4}
    0xE925_0006, // stmdb r5!, {r1,r2}
    0xE8A0_0003, // stmia r0!, {r1,r0} (stored-base quirk)
    0xE8B0_0009, // ldmia r0, {r0,r3} (base in list: no writeback)
    0xE3A0_7007, // mov r7, #7
];
const ARM_BLOCK_REGS: [(usize, u32); 5] = [
    (0, 0x0200_0000),
    (1, 0x1111_1111),
    (2, 0x2222_2222),
    (5, 0x0200_0000),
    (6, 0x0300_0400),
];

#[test]
fn micro_op_arm_block_matches_legacy() {
    for waitcnt in [0x0000u16, 0x0010, 0x4000, 0x4010, 0x4014] {
        let (ta, tb, regs_equal, follow) = differential(
            &ARM_BLOCK_CORPUS,
            false,
            waitcnt,
            ARM_BLOCK_CORPUS.len(),
            0xE1DD_20B0,
            0x0300_0000,
            None,
            &[],
            &ARM_BLOCK_REGS,
        );
        assert_eq!((ta, tb), (ta, ta), "iwram waitcnt={waitcnt:#06x}");
        assert!(regs_equal, "iwram waitcnt={waitcnt:#06x}");
        assert!(follow, "iwram waitcnt={waitcnt:#06x}");
    }
    // ROM-code variant: data still hits EWRAM, batch erase is the
    // ROM-code whole-word path (fetch_width 4).
    for waitcnt in [0x0000u16, 0x4010, 0x4014] {
        let (ta, tb, regs_equal, follow) = differential(
            &ARM_BLOCK_CORPUS,
            false,
            waitcnt,
            ARM_BLOCK_CORPUS.len(),
            0xE1DD_20B0,
            0x0800_0100,
            Some(rom_cart()),
            &[],
            &ARM_BLOCK_REGS,
        );
        assert_eq!((ta, tb), (ta, ta), "rom waitcnt={waitcnt:#06x}");
        assert!(regs_equal, "rom waitcnt={waitcnt:#06x}");
        assert!(follow, "rom waitcnt={waitcnt:#06x}");
    }
}

#[test]
fn arm_block_gates_stay_legacy() {
    let mut regs = CpuRegisters::post_bios();
    regs.set_r(0, 0x0200_0000);
    // S bit now expands (user-bank forms, asserted below); only the
    // empty list stays legacy.
    let ops = expand_arm(0xE8B5_0018 | (1 << 22), &regs).expect("ldmia^ expands");
    // BlockStart + 2 words + BlockEnd + 2 trailing = 6.
    assert_eq!(ops.len(), 6);
    // Empty list (plain and S-bit).
    assert!(expand_arm(0xE8A0_0000, &regs).is_none());
    assert!(expand_arm(0xE8A0_0000 | (1 << 22), &regs).is_none());
    let ops = expand_arm(0xE8A0_0006, &regs).expect("plain stmia expands");
    // BlockStart + 2 words + BlockEnd + 1 trailing = 5.
    assert_eq!(ops.len(), 5);
}

/// ARM block memory effects under both engines: cell words as well
/// as registers and totals must agree.
#[test]
fn arm_block_memory_matches_legacy() {
    fn setup() -> (GbaCpu, GbaMemoryBus) {
        let mut cpu = GbaCpu::post_bios();
        let mut bus = GbaMemoryBus::new();
        for (i, w) in ARM_BLOCK_CORPUS.iter().enumerate() {
            bus.write32(0x0300_0000 + (i as u32) * 4, *w);
        }
        for (r, v) in ARM_BLOCK_REGS {
            cpu.regs.set_r(r, v);
        }
        cpu.regs.set_pc(0x0300_0000);
        fill_pipeline(&mut cpu.regs, &mut bus, &mut cpu.pipeline);
        bus.take_access_wait_cycles();
        (cpu, bus)
    }
    fn drain_micro(cpu: &mut GbaCpu, bus: &mut GbaMemoryBus, steps: usize) -> u32 {
        let mut total = 0u32;
        let mut queue = std::collections::VecDeque::new();
        for _ in 0..steps {
            let mut acc = 0i64;
            loop {
                acc += step_op(&mut cpu.regs, bus, &mut cpu.pipeline, &mut queue, false)
                    .expect("corpus must be covered");
                if queue.is_empty() {
                    break;
                }
            }
            total += acc.max(1) as u32;
        }
        total
    }
    let (mut a_cpu, mut a_bus) = setup();
    let (mut b_cpu, mut b_bus) = setup();
    let mut ta = 0u32;
    for _ in 0..ARM_BLOCK_CORPUS.len() {
        ta += a_cpu.step_legacy(&mut a_bus);
    }
    let tb = drain_micro(&mut b_cpu, &mut b_bus, ARM_BLOCK_CORPUS.len());
    assert_eq!((ta, tb), (ta, ta));
    for r in 0..16 {
        assert_eq!(a_cpu.regs.r(r), b_cpu.regs.r(r), "r{r}");
    }
    assert_eq!(a_cpu.regs.cpsr(), b_cpu.regs.cpsr());
    for addr in (0x0200_0000..0x0200_0020).step_by(4) {
        assert_eq!(a_bus.read32(addr), b_bus.read32(addr), "cell {addr:#010X}");
    }
}

/// ARM LDM with PC in the list: PC lands word-aligned, the retire
/// flushes the pipeline at the target (+ ARM lead).
#[test]
fn arm_ldm_pc_matches_legacy_and_flushes() {
    fn setup() -> (GbaCpu, GbaMemoryBus) {
        let mut cpu = GbaCpu::post_bios();
        let mut bus = GbaMemoryBus::new();
        bus.write32(0x0300_0000, 0xE890_8000); // ldmia r0, {r15} (no writeback)
        bus.write32(0x0200_0000, 0x0200_0100);
        bus.write32(0x0200_0100, 0xE3A0_7007); // mov r7, #7 (target)
        cpu.regs.set_r(0, 0x0200_0000);
        cpu.regs.set_pc(0x0300_0000);
        fill_pipeline(&mut cpu.regs, &mut bus, &mut cpu.pipeline);
        bus.take_access_wait_cycles();
        (cpu, bus)
    }
    let (mut a_cpu, mut a_bus) = setup();
    let ta = a_cpu.step_legacy(&mut a_bus);
    let (mut b_cpu, mut b_bus) = setup();
    let mut queue = std::collections::VecDeque::new();
    let mut acc = 0i64;
    loop {
        acc += step_op(
            &mut b_cpu.regs,
            &mut b_bus,
            &mut b_cpu.pipeline,
            &mut queue,
            false,
        )
        .expect("ldm-pc must be covered");
        if queue.is_empty() {
            break;
        }
    }
    let tb = acc.max(1) as u32;
    assert_eq!((ta, tb), (ta, ta));
    assert_eq!(b_cpu.regs.pc(), 0x0200_0108); // target + ARM pipeline lead
    assert_eq!(b_cpu.regs.pc(), a_cpu.regs.pc());
    assert_eq!(b_cpu.regs.r(0), 0x0200_0000);
    assert_eq!(b_cpu.regs.r(0), a_cpu.regs.r(0));
}

#[test]
fn arm_block_tick_parity() {
    let (at, bt, av, bv) = tick_parity(
        &ARM_BLOCK_CORPUS,
        false,
        0x4014,
        0x0300_0000,
        &ARM_BLOCK_REGS,
        ARM_BLOCK_CORPUS.len(),
    );
    assert_eq!((at, bt), (at, at), "ticks diverge");
    assert_eq!((av, bv), (av, av), "timer span diverges");
}

// Slice 6 corpus: Thumb LDR (literal). Both loads target the
// pool word at 0x0C (the second sits at pc=0x06, pinning the
// `pc & !3` alignment); the tail never executes the pool.
const THUMB_PCREL_CORPUS: [u32; 6] = [
    0x4801, // ldr r0, [pc, #4] -> 0x0C
    0x4901, // ldr r1, [pc, #4] -> 0x0C (pc=0x06, aligned down)
    0x2707, // mov r7, #7
    0x2000, // (padding)
    0xBEEF, // pool lo
    0xDEAD, // pool hi -> 0xDEADBEEF
];

#[test]
fn micro_op_pcrel_matches_legacy() {
    for waitcnt in [0x0000u16, 0x0010, 0x4000, 0x4010, 0x4014] {
        let (ta, tb, regs_equal, follow) = differential(
            &THUMB_PCREL_CORPUS,
            true,
            waitcnt,
            3,
            0x886A,
            0x0300_0000,
            None,
            &[],
            &[],
        );
        assert_eq!((ta, tb), (ta, ta), "iwram waitcnt={waitcnt:#06x}");
        assert!(regs_equal, "iwram waitcnt={waitcnt:#06x}");
        assert!(follow, "iwram waitcnt={waitcnt:#06x}");
    }
    // ROM-code variant: pool word in ROM (prefetch-window path).
    for waitcnt in [0x0000u16, 0x4010, 0x4014] {
        let (ta, tb, regs_equal, follow) = differential(
            &THUMB_PCREL_CORPUS,
            true,
            waitcnt,
            3,
            0x886A,
            0x0800_0100,
            Some(rom_cart()),
            &[],
            &[],
        );
        assert_eq!((ta, tb), (ta, ta), "rom waitcnt={waitcnt:#06x}");
        assert!(regs_equal, "rom waitcnt={waitcnt:#06x}");
        assert!(follow, "rom waitcnt={waitcnt:#06x}");
    }
}

/// LDR-literal value and alignment: an odd-halfword instruction
/// must align pc down before adding the offset.
#[test]
fn pcrel_load_value_and_align() {
    fn setup() -> (GbaCpu, GbaMemoryBus) {
        let mut cpu = GbaCpu::post_bios();
        let mut bus = GbaMemoryBus::new();
        bus.write16(0x0300_0000, 0x4800); // (padding)
        bus.write16(0x0300_0002, 0x4901); // ldr r1, [pc, #4]
        bus.write32(0x0300_0008, 0xCAFE_BABE);
        cpu.regs.set_cpsr(cpu.regs.cpsr() | (1 << 5));
        cpu.regs.set_pc(0x0300_0002);
        fill_pipeline(&mut cpu.regs, &mut bus, &mut cpu.pipeline);
        bus.take_access_wait_cycles();
        (cpu, bus)
    }
    // pc=0x06 at execute, &!3=0x04, +4 -> 0x08 pool.
    let (mut a_cpu, mut a_bus) = setup();
    let ta = a_cpu.step_legacy(&mut a_bus);
    let (mut b_cpu, mut b_bus) = setup();
    let mut queue = std::collections::VecDeque::new();
    let mut acc = 0i64;
    loop {
        acc += step_op(
            &mut b_cpu.regs,
            &mut b_bus,
            &mut b_cpu.pipeline,
            &mut queue,
            true,
        )
        .expect("pcrel must be covered");
        if queue.is_empty() {
            break;
        }
    }
    let tb = acc.max(1) as u32;
    assert_eq!((ta, tb), (ta, ta));
    assert_eq!(a_cpu.regs.r(1), 0xCAFE_BABE);
    assert_eq!(b_cpu.regs.r(1), 0xCAFE_BABE);
}

#[test]
fn pcrel_tick_parity() {
    let (at, bt, av, bv) = tick_parity(&THUMB_PCREL_CORPUS, true, 0x0000, 0x0300_0000, &[], 3);
    assert_eq!((at, bt), (at, at), "ticks diverge");
    assert_eq!((av, bv), (av, av), "timer span diverges");
}

// Slice 7 corpus: ARM data-processing (register form). Immediate
// shifts (LSL/LSR), a register shift (+1I), flag-only TST, and S
// set/clear across opcodes.
const ARM_DPREG_CORPUS: [u32; 7] = [
    0xE1A0_0001, // mov r0, r1
    0xE081_0002, // add r0, r1, r2
    0xE051_0002, // subs r0, r1, r2
    0xE111_0002, // tst r1, r2
    0xE1B0_0213, // mov r0, r3, lsl r2 (register shift)
    0xE1A0_0122, // mov r0, r2, lsr #2
    0xE3A0_3005, // mov r3, #5
];
const ARM_DPREG_REGS: [(usize, u32); 3] = [(1, 0x100), (2, 4), (3, 0xFF)];

#[test]
fn micro_op_arm_dpreg_matches_legacy() {
    for waitcnt in [0x0000u16, 0x0010, 0x4000, 0x4010, 0x4014] {
        let (ta, tb, regs_equal, follow) = differential(
            &ARM_DPREG_CORPUS,
            false,
            waitcnt,
            ARM_DPREG_CORPUS.len(),
            0xE1DD_20B0,
            0x0300_0000,
            None,
            &[],
            &ARM_DPREG_REGS,
        );
        assert_eq!((ta, tb), (ta, ta), "iwram waitcnt={waitcnt:#06x}");
        assert!(regs_equal, "iwram waitcnt={waitcnt:#06x}");
        assert!(follow, "iwram waitcnt={waitcnt:#06x}");
    }
    for waitcnt in [0x0000u16, 0x4010, 0x4014] {
        let (ta, tb, regs_equal, follow) = differential(
            &ARM_DPREG_CORPUS,
            false,
            waitcnt,
            ARM_DPREG_CORPUS.len(),
            0xE1DD_20B0,
            0x0800_0100,
            Some(rom_cart()),
            &[],
            &ARM_DPREG_REGS,
        );
        assert_eq!((ta, tb), (ta, ta), "rom waitcnt={waitcnt:#06x}");
        assert!(regs_equal, "rom waitcnt={waitcnt:#06x}");
        assert!(follow, "rom waitcnt={waitcnt:#06x}");
    }
}

#[test]
fn arm_dpreg_gates_stay_legacy() {
    let regs = CpuRegisters::post_bios();
    // The DP-register branch itself must not misclaim multiply,
    // SWP, MRS/MSR, BX or halfword shapes (they have their own
    // branches or stay legacy); assert on the branch directly.
    for instr in [
        0xE002_0091, // mul
        0xE102_0091, // swp
        0xE10F_0000, // mrs
        0xE129_F000, // msr
        0xE12F_FF11, // bx
        0xE112_00F3, // ldrsh-reg
    ] {
        assert!(expand_arm_dp_reg(instr, &regs).is_none(), "{instr:#010X}");
    }
    let ops = expand_arm(0xE081_0002, &regs).expect("add-reg expands");
    // Commit + 0 trailing = 1.
    assert_eq!(ops.len(), 1);
    let ops = expand_arm(0xE1B0_0213, &regs).expect("reg-shift expands");
    // Commit + 1I = 2.
    assert_eq!(ops.len(), 2);
}

#[test]
fn arm_dpreg_tick_parity() {
    let (at, bt, av, bv) = tick_parity(
        &ARM_DPREG_CORPUS,
        false,
        0x4014,
        0x0300_0000,
        &ARM_DPREG_REGS,
        ARM_DPREG_CORPUS.len(),
    );
    assert_eq!((at, bt), (at, at), "ticks diverge");
    assert_eq!((av, bv), (av, av), "timer span diverges");
}

// Slice 8 corpus: ARM multiply (short MUL/MLA/S, long UMULL).
const ARM_MUL_CORPUS: [u32; 5] = [
    0xE000_0291, // mul r0, r1, r2
    0xE022_0391, // mla r2, r1, r3, r0
    0xE001_0291, // muls r0, r1, r2
    0xE083_2190, // umull r3, r2, r0, r1
    0xE3A0_7007, // mov r7, #7
];
const ARM_MUL_REGS: [(usize, u32); 4] = [(0, 3), (1, 4), (2, 7), (3, 5)];

#[test]
fn micro_op_arm_mul_matches_legacy() {
    for waitcnt in [0x0000u16, 0x0010, 0x4000, 0x4010, 0x4014] {
        let (ta, tb, regs_equal, follow) = differential(
            &ARM_MUL_CORPUS,
            false,
            waitcnt,
            ARM_MUL_CORPUS.len(),
            0xE1DD_20B0,
            0x0300_0000,
            None,
            &[],
            &ARM_MUL_REGS,
        );
        assert_eq!((ta, tb), (ta, ta), "iwram waitcnt={waitcnt:#06x}");
        assert!(regs_equal, "iwram waitcnt={waitcnt:#06x}");
        assert!(follow, "iwram waitcnt={waitcnt:#06x}");
    }
    for waitcnt in [0x0000u16, 0x4010, 0x4014] {
        let (ta, tb, regs_equal, follow) = differential(
            &ARM_MUL_CORPUS,
            false,
            waitcnt,
            ARM_MUL_CORPUS.len(),
            0xE1DD_20B0,
            0x0800_0100,
            Some(rom_cart()),
            &[],
            &ARM_MUL_REGS,
        );
        assert_eq!((ta, tb), (ta, ta), "rom waitcnt={waitcnt:#06x}");
        assert!(regs_equal, "rom waitcnt={waitcnt:#06x}");
        assert!(follow, "rom waitcnt={waitcnt:#06x}");
    }
}

#[test]
fn arm_mul_padding_matches_legacy_base() {
    let mut regs = CpuRegisters::post_bios();
    regs.set_r(2, 7);
    // MUL m=1: commit + 1I.
    let ops = expand_arm(0xE000_0291, &regs).expect("mul expands");
    assert_eq!(ops.len(), 2);
    // MLA m=1: commit + 2I.
    let ops = expand_arm(0xE022_0391, &regs).expect("mla expands");
    assert_eq!(ops.len(), 3);
    // UMULL ticks=1: commit + 2I.
    let ops = expand_arm(0xE083_2190, &regs).expect("umull expands");
    assert_eq!(ops.len(), 3);
    // Full-width multiplier: m=4.
    regs.set_r(2, 0x8000_0000);
    let ops = expand_arm(0xE000_0291, &regs).expect("wide mul expands");
    assert_eq!(ops.len(), 5);
    // Plain DP beside the masks stays on its own path.
    assert!(expand_arm_mul(0xE081_0002, &regs).is_none());
}

#[test]
fn arm_mul_tick_parity() {
    let (at, bt, av, bv) = tick_parity(
        &ARM_MUL_CORPUS,
        false,
        0x4014,
        0x0300_0000,
        &ARM_MUL_REGS,
        ARM_MUL_CORPUS.len(),
    );
    assert_eq!((at, bt), (at, at), "ticks diverge");
    assert_eq!((av, bv), (av, av), "timer span diverges");
}

// Thumb MUL corpus (m from the incoming Rd).
const THUMB_MUL_CORPUS: [u32; 3] = [
    0x4341, // mul r1, r0
    0x434A, // mul r2, r1
    0x2707, // mov r7, #7
];
const THUMB_MUL_REGS: [(usize, u32); 3] = [(0, 3), (1, 4), (2, 5)];

#[test]
fn micro_op_thumb_mul_matches_legacy() {
    for waitcnt in [0x0000u16, 0x0010, 0x4000, 0x4010, 0x4014] {
        let (ta, tb, regs_equal, follow) = differential(
            &THUMB_MUL_CORPUS,
            true,
            waitcnt,
            THUMB_MUL_CORPUS.len(),
            0x886A,
            0x0300_0000,
            None,
            &[],
            &THUMB_MUL_REGS,
        );
        assert_eq!((ta, tb), (ta, ta), "iwram waitcnt={waitcnt:#06x}");
        assert!(regs_equal, "iwram waitcnt={waitcnt:#06x}");
        assert!(follow, "iwram waitcnt={waitcnt:#06x}");
    }
    for waitcnt in [0x0000u16, 0x4010, 0x4014] {
        let (ta, tb, regs_equal, follow) = differential(
            &THUMB_MUL_CORPUS,
            true,
            waitcnt,
            THUMB_MUL_CORPUS.len(),
            0x886A,
            0x0800_0100,
            Some(rom_cart()),
            &[],
            &THUMB_MUL_REGS,
        );
        assert_eq!((ta, tb), (ta, ta), "rom waitcnt={waitcnt:#06x}");
        assert!(regs_equal, "rom waitcnt={waitcnt:#06x}");
        assert!(follow, "rom waitcnt={waitcnt:#06x}");
    }
}

#[test]
fn thumb_mul_gate_and_padding() {
    let mut regs = CpuRegisters::post_bios();
    regs.set_cpsr(regs.cpsr() | (1 << 5));
    regs.set_r(1, 4);
    // AND beside the MUL encoding is covered by the ALU-remainder
    // branch (asserted in thumb_alu_rest_shapes_and_gates).
    // MUL m=1 (Rd=4): commit + 1I.
    let ops = expand_thumb(0x4341, &regs).expect("thumb mul expands");
    assert_eq!(ops.len(), 2);
}

#[test]
fn thumb_mul_tick_parity() {
    let (at, bt, av, bv) = tick_parity(
        &THUMB_MUL_CORPUS,
        true,
        0x0000,
        0x0300_0000,
        &THUMB_MUL_REGS,
        THUMB_MUL_CORPUS.len(),
    );
    assert_eq!((at, bt), (at, at), "ticks diverge");
    assert_eq!((av, bv), (av, av), "timer span diverges");
}

// Slice 9 corpus: Thumb long-BL call and BX return. The call
// targets 0x10 (high 0xF000, low 0xF806); the subroutine returns
// via BX LR to the MOV, which then skips over the subroutine.
const THUMB_BL_CORPUS: [u32; 11] = [
    0xF000, // bl-hi
    0xF806, // bl-lo (target 0x10, LR=0x05)
    0x2707, // mov r7, #7 (return landing)
    0xE005, // b 0x14
    0x2000, // (padding)
    0x2000, // (padding)
    0x2000, // (padding)
    0x2000, // (padding)
    0x2001, // 0x10: mov r0, #1
    0x4770, // bx lr
    0x2102, // 0x14: mov r1, #2
];

#[test]
fn micro_op_thumb_bl_bx_matches_legacy() {
    for waitcnt in [0x0000u16, 0x0010, 0x4000, 0x4010, 0x4014] {
        let (ta, tb, regs_equal, follow) = differential(
            &THUMB_BL_CORPUS,
            true,
            waitcnt,
            6,
            0x886A,
            0x0300_0000,
            None,
            &[],
            &[],
        );
        assert_eq!((ta, tb), (ta, ta), "iwram waitcnt={waitcnt:#06x}");
        assert!(regs_equal, "iwram waitcnt={waitcnt:#06x}");
        assert!(follow, "iwram waitcnt={waitcnt:#06x}");
    }
    for waitcnt in [0x0000u16, 0x4010, 0x4014] {
        let (ta, tb, regs_equal, follow) = differential(
            &THUMB_BL_CORPUS,
            true,
            waitcnt,
            6,
            0x886A,
            0x0800_0100,
            Some(rom_cart()),
            &[],
            &[],
        );
        assert_eq!((ta, tb), (ta, ta), "rom waitcnt={waitcnt:#06x}");
        assert!(regs_equal, "rom waitcnt={waitcnt:#06x}");
        assert!(follow, "rom waitcnt={waitcnt:#06x}");
    }
}

#[test]
fn thumb_bl_bx_shapes_and_gates() {
    let mut regs = CpuRegisters::post_bios();
    regs.set_cpsr(regs.cpsr() | (1 << 5));
    regs.set_lr(0x0300_0005);
    // BL-high: single LR op. BL-low / BX: refill pair + commit.
    let bl_hi = expand_thumb(0xF000, &regs).expect("bl-hi expands");
    assert_eq!(bl_hi.len(), 1);
    let bl_lo = expand_thumb(0xF806, &regs).expect("bl-lo expands");
    assert_eq!(bl_lo.len(), 3);
    let bx = expand_thumb(0x4770, &regs).expect("bx expands");
    assert_eq!(bx.len(), 3);
    // Hi-reg ADD beside BX is covered by the ALU-remainder branch
    // (asserted in thumb_alu_rest_shapes_and_gates).
}

/// BX to an ARM (even) target switches mode; the retire refills
/// with the ARM width (+8 lead).
#[test]
fn bx_switches_mode_like_legacy() {
    fn setup() -> (GbaCpu, GbaMemoryBus) {
        let mut cpu = GbaCpu::post_bios();
        let mut bus = GbaMemoryBus::new();
        bus.write16(0x0300_0000, 0x4708); // bx r1
        bus.write32(0x0200_0100, 0xE3A0_7007); // mov r7, #7 (ARM target)
        cpu.regs.set_cpsr(cpu.regs.cpsr() | (1 << 5));
        cpu.regs.set_r(1, 0x0200_0100);
        cpu.regs.set_pc(0x0300_0000);
        fill_pipeline(&mut cpu.regs, &mut bus, &mut cpu.pipeline);
        bus.take_access_wait_cycles();
        (cpu, bus)
    }
    let (mut a_cpu, mut a_bus) = setup();
    let ta = a_cpu.step_legacy(&mut a_bus);
    let (mut b_cpu, mut b_bus) = setup();
    let mut queue = std::collections::VecDeque::new();
    let mut acc = 0i64;
    loop {
        acc += step_op(
            &mut b_cpu.regs,
            &mut b_bus,
            &mut b_cpu.pipeline,
            &mut queue,
            true,
        )
        .expect("bx must be covered");
        if queue.is_empty() {
            break;
        }
    }
    let tb = acc.max(1) as u32;
    assert_eq!((ta, tb), (ta, ta));
    assert_eq!(b_cpu.regs.pc(), 0x0200_0108);
    assert_eq!(b_cpu.regs.pc(), a_cpu.regs.pc());
    assert!(!b_cpu.regs.cpsr_t());
    assert_eq!(b_cpu.regs.cpsr_t(), a_cpu.regs.cpsr_t());
}

#[test]
fn thumb_bl_bx_tick_parity() {
    let (at, bt, av, bv) = tick_parity(&THUMB_BL_CORPUS, true, 0x0000, 0x0300_0000, &[], 6);
    assert_eq!((at, bt), (at, at), "ticks diverge");
    assert_eq!((av, bv), (av, av), "timer span diverges");
}

// Slice 10 corpus: Thumb ALU remainder (shifts, ADD/SUB,
// reg-ALU incl. ROR, hi-reg CMP/ADD).
const THUMB_ALU_CORPUS: [u32; 10] = [
    0x0041, // lsl r1, r0, #1
    0x0801, // lsr r1, r0, #32
    0x1042, // asr r2, r0, #1
    0x1881, // add r1, r0, r2
    0x1EC1, // sub r1, r0, #3
    0x4001, // and r1, r0
    0x41C1, // ror r1, r0
    0x4580, // cmp r8, r0
    0x4480, // add r8, r0
    0x2707, // mov r7, #7
];
const THUMB_ALU_REGS: [(usize, u32); 5] =
    [(0, 0x8000_0001), (1, 0x100), (2, 4), (3, 0xFF), (8, 0x10)];

#[test]
fn micro_op_thumb_alu_rest_matches_legacy() {
    for waitcnt in [0x0000u16, 0x0010, 0x4000, 0x4010, 0x4014] {
        let (ta, tb, regs_equal, follow) = differential(
            &THUMB_ALU_CORPUS,
            true,
            waitcnt,
            THUMB_ALU_CORPUS.len(),
            0x886A,
            0x0300_0000,
            None,
            &[],
            &THUMB_ALU_REGS,
        );
        assert_eq!((ta, tb), (ta, ta), "iwram waitcnt={waitcnt:#06x}");
        assert!(regs_equal, "iwram waitcnt={waitcnt:#06x}");
        assert!(follow, "iwram waitcnt={waitcnt:#06x}");
    }
    for waitcnt in [0x0000u16, 0x4010, 0x4014] {
        let (ta, tb, regs_equal, follow) = differential(
            &THUMB_ALU_CORPUS,
            true,
            waitcnt,
            THUMB_ALU_CORPUS.len(),
            0x886A,
            0x0800_0100,
            Some(rom_cart()),
            &[],
            &THUMB_ALU_REGS,
        );
        assert_eq!((ta, tb), (ta, ta), "rom waitcnt={waitcnt:#06x}");
        assert!(regs_equal, "rom waitcnt={waitcnt:#06x}");
        assert!(follow, "rom waitcnt={waitcnt:#06x}");
    }
}

#[test]
fn thumb_alu_rest_shapes_and_gates() {
    let mut regs = CpuRegisters::post_bios();
    regs.set_cpsr(regs.cpsr() | (1 << 5));
    // Single-cycle forms: commit only.
    for instr in [0x0041, 0x1881, 0x4001, 0x4580] {
        let ops = expand_thumb(instr, &regs).expect("alu form expands");
        assert_eq!(ops.len(), 1, "{instr:#06X}");
    }
    // Register shift: commit + 1I. ADD PC: commit + 2I.
    let ops = expand_thumb(0x41C1, &regs).expect("ror expands");
    assert_eq!(ops.len(), 2);
    let ops = expand_thumb(0x4487, &regs).expect("add-pc expands");
    assert_eq!(ops.len(), 3);
    // MUL and BX keep their own branches; SWI stays legacy.
    assert!(expand_thumb_alu_rest(0x4341).is_none());
    assert!(expand_thumb_alu_rest(0x4708).is_none());
    assert!(expand_thumb(0xDF00, &regs).is_none());
}

#[test]
fn thumb_alu_rest_tick_parity() {
    let (at, bt, av, bv) = tick_parity(
        &THUMB_ALU_CORPUS,
        true,
        0x0000,
        0x0300_0000,
        &THUMB_ALU_REGS,
        THUMB_ALU_CORPUS.len(),
    );
    assert_eq!((at, bt), (at, at), "ticks diverge");
    assert_eq!((av, bv), (av, av), "timer span diverges");
}

// Slice 11 corpus: Thumb ADD SP/PC and SP offset (bus-free).
const THUMB_SPADD_CORPUS: [u32; 5] = [
    0xA004, // add r0, pc, #0x10
    0xA910, // add r1, sp, #0x40
    0xB00A, // add sp, #0x28
    0xB08A, // sub sp, #0x28
    0x2707, // mov r7, #7
];

#[test]
fn micro_op_thumb_spadd_matches_legacy() {
    for waitcnt in [0x0000u16, 0x0010, 0x4000, 0x4010, 0x4014] {
        let (ta, tb, regs_equal, follow) = differential(
            &THUMB_SPADD_CORPUS,
            true,
            waitcnt,
            THUMB_SPADD_CORPUS.len(),
            0x886A,
            0x0300_0000,
            None,
            &[],
            &[],
        );
        assert_eq!((ta, tb), (ta, ta), "iwram waitcnt={waitcnt:#06x}");
        assert!(regs_equal, "iwram waitcnt={waitcnt:#06x}");
        assert!(follow, "iwram waitcnt={waitcnt:#06x}");
    }
    for waitcnt in [0x0000u16, 0x4010, 0x4014] {
        let (ta, tb, regs_equal, follow) = differential(
            &THUMB_SPADD_CORPUS,
            true,
            waitcnt,
            THUMB_SPADD_CORPUS.len(),
            0x886A,
            0x0800_0100,
            Some(rom_cart()),
            &[],
            &[],
        );
        assert_eq!((ta, tb), (ta, ta), "rom waitcnt={waitcnt:#06x}");
        assert!(regs_equal, "rom waitcnt={waitcnt:#06x}");
        assert!(follow, "rom waitcnt={waitcnt:#06x}");
    }
}

// Slice 11b: ARM ALU-immediate with imm bit4 set (previously gated
// to legacy; I==1 routes to DP or Rd==15 PSR-imm, both safe).
const ARM_IMM4_CORPUS: [u32; 4] = [
    0xE3A0_00FF, // mov r0, #0xFF
    0xE281_101F, // add r1, r1, #0x1F
    0xE252_20F0, // subs r2, r2, #0xF0
    0xE3A0_3005, // mov r3, #5
];

#[test]
fn micro_op_arm_imm_bit4_matches_legacy() {
    for waitcnt in [0x0000u16, 0x4010, 0x4014] {
        let (ta, tb, regs_equal, follow) = differential(
            &ARM_IMM4_CORPUS,
            false,
            waitcnt,
            ARM_IMM4_CORPUS.len(),
            0xE1DD_20B0,
            0x0300_0000,
            None,
            &[],
            &[(1, 1), (2, 0x100)],
        );
        assert_eq!((ta, tb), (ta, ta), "iwram waitcnt={waitcnt:#06x}");
        assert!(regs_equal, "iwram waitcnt={waitcnt:#06x}");
        assert!(follow, "iwram waitcnt={waitcnt:#06x}");
        let (ta, tb, regs_equal, follow) = differential(
            &ARM_IMM4_CORPUS,
            false,
            waitcnt,
            ARM_IMM4_CORPUS.len(),
            0xE1DD_20B0,
            0x0800_0100,
            Some(rom_cart()),
            &[],
            &[(1, 1), (2, 0x100)],
        );
        assert_eq!((ta, tb), (ta, ta), "rom waitcnt={waitcnt:#06x}");
        assert!(regs_equal, "rom waitcnt={waitcnt:#06x}");
        assert!(follow, "rom waitcnt={waitcnt:#06x}");
    }
}

#[test]
fn spadd_shapes_and_imm4_gate() {
    let mut regs = CpuRegisters::post_bios();
    regs.set_cpsr(regs.cpsr() | (1 << 5));
    for instr in [0xA004, 0xA910, 0xB00A, 0xB08A] {
        let ops = expand_thumb(instr, &regs).expect("sp-add expands");
        assert_eq!(ops.len(), 1, "{instr:#06X}");
    }
    // Imm bit4 no longer gates the ARM immediate form.
    let ops = expand_arm(0xE3A0_00FF, &regs).expect("bit4 imm expands");
    assert_eq!(ops.len(), 1);
    // MSR-immediate routes to the PSR branch (not DP-imm); MOV
    // with Rd==15 (S=0) expands with refill padding.
    let ops = expand_arm(0xE329_F000, &regs).expect("msr-imm expands");
    assert_eq!(ops.len(), 1);
    let ops = expand_arm(0xE3A0_F005, &regs).expect("mov-pc expands");
    assert_eq!(ops.len(), 3);
}

#[test]
fn spadd_tick_parity() {
    let (at, bt, av, bv) = tick_parity(&THUMB_SPADD_CORPUS, true, 0x0000, 0x0300_0000, &[], 5);
    assert_eq!((at, bt), (at, at), "ticks diverge");
    assert_eq!((av, bv), (av, av), "timer span diverges");
}

// Slice 12 corpus: ARM single-transfer remainder (register
// offsets incl. shift, R15 base/dest, halfword-imm R15). Cell A is
// EWRAM; the [pc] loads read code words as data (deterministic).
const ARM_SINGLEREST_CORPUS: [u32; 8] = [
    0xE791_0002, // ldr r0, [r1, r2]
    0xE7C1_3004, // strb r3, [r1, r4]
    0xE790_2104, // ldr r2, [r0, r4, lsl #2]
    0xE59F_5000, // ldr r5, [pc]
    0xE581_F004, // str r15, [r1, #4]
    0xE1DF_00B0, // ldrh r0, [pc]
    0xE1C1_F0B0, // strh r15, [r1]
    0xE3A0_7007, // mov r7, #7
];
const ARM_SINGLEREST_MEM: [(u32, u8, u32); 3] = [
    (0x0200_0000, 4, 0xDEAD_0001),
    (0x0200_0004, 4, 0x0200_0020),
    (0x0200_0020, 4, 0xCAFE_BABE),
];
const ARM_SINGLEREST_REGS: [(usize, u32); 5] =
    [(1, 0x0200_0000), (2, 4), (3, 0x1234_5678), (4, 0), (5, 0)];

#[test]
fn micro_op_arm_singlerest_matches_legacy() {
    for waitcnt in [0x0000u16, 0x0010, 0x4000, 0x4010, 0x4014] {
        let (ta, tb, regs_equal, follow) = differential(
            &ARM_SINGLEREST_CORPUS,
            false,
            waitcnt,
            ARM_SINGLEREST_CORPUS.len(),
            0xE1DD_20B0,
            0x0300_0000,
            None,
            &ARM_SINGLEREST_MEM,
            &ARM_SINGLEREST_REGS,
        );
        assert_eq!((ta, tb), (ta, ta), "iwram waitcnt={waitcnt:#06x}");
        assert!(regs_equal, "iwram waitcnt={waitcnt:#06x}");
        assert!(follow, "iwram waitcnt={waitcnt:#06x}");
    }
    for waitcnt in [0x0000u16, 0x4010, 0x4014] {
        let (ta, tb, regs_equal, follow) = differential(
            &ARM_SINGLEREST_CORPUS,
            false,
            waitcnt,
            ARM_SINGLEREST_CORPUS.len(),
            0xE1DD_20B0,
            0x0800_0100,
            Some(rom_cart()),
            &ARM_SINGLEREST_MEM,
            &ARM_SINGLEREST_REGS,
        );
        assert_eq!((ta, tb), (ta, ta), "rom waitcnt={waitcnt:#06x}");
        assert!(regs_equal, "rom waitcnt={waitcnt:#06x}");
        assert!(follow, "rom waitcnt={waitcnt:#06x}");
    }
}

#[test]
fn arm_singlerest_shapes() {
    let mut regs = CpuRegisters::post_bios();
    regs.set_r(1, 0x0200_0000);
    regs.set_r(2, 4);
    // Register offset: [Read, I, I].
    let ops = expand_arm(0xE791_0002, &regs).expect("reg-offset expands");
    assert_eq!(ops.len(), 3);
    // R15 load: +2 refill. R15 store: plain [Write, I].
    let ops = expand_arm(0xE59F_F000, &regs).expect("ldr-pc expands");
    assert_eq!(ops.len(), 5);
    let ops = expand_arm(0xE581_F004, &regs).expect("str-r15 expands");
    assert_eq!(ops.len(), 2);
    let ops = expand_arm(0xE1DF_00B0, &regs).expect("ldrh-pc expands");
    assert_eq!(ops.len(), 3);
}

/// Single-transfer memory effects under both engines: stored words
/// (incl. STR R15 snapshots) as well as registers and totals agree.
#[test]
fn arm_singlerest_memory_matches_legacy() {
    fn setup() -> (GbaCpu, GbaMemoryBus) {
        let mut cpu = GbaCpu::post_bios();
        let mut bus = GbaMemoryBus::new();
        for (i, w) in ARM_SINGLEREST_CORPUS.iter().enumerate() {
            bus.write32(0x0300_0000 + (i as u32) * 4, *w);
        }
        for (addr, width, val) in ARM_SINGLEREST_MEM {
            match width {
                4 => bus.write32(addr, val),
                2 => bus.write16(addr, (val & 0xFFFF) as u16),
                _ => bus.write8(addr, (val & 0xFF) as u8),
            }
        }
        for (r, v) in ARM_SINGLEREST_REGS {
            cpu.regs.set_r(r, v);
        }
        cpu.regs.set_pc(0x0300_0000);
        fill_pipeline(&mut cpu.regs, &mut bus, &mut cpu.pipeline);
        bus.take_access_wait_cycles();
        (cpu, bus)
    }
    fn drain_micro(cpu: &mut GbaCpu, bus: &mut GbaMemoryBus, steps: usize) -> u32 {
        let mut total = 0u32;
        let mut queue = std::collections::VecDeque::new();
        for _ in 0..steps {
            let mut acc = 0i64;
            loop {
                acc += step_op(&mut cpu.regs, bus, &mut cpu.pipeline, &mut queue, false)
                    .expect("corpus must be covered");
                if queue.is_empty() {
                    break;
                }
            }
            total += acc.max(1) as u32;
        }
        total
    }
    let (mut a_cpu, mut a_bus) = setup();
    let (mut b_cpu, mut b_bus) = setup();
    let mut ta = 0u32;
    for _ in 0..ARM_SINGLEREST_CORPUS.len() {
        ta += a_cpu.step_legacy(&mut a_bus);
    }
    let tb = drain_micro(&mut b_cpu, &mut b_bus, ARM_SINGLEREST_CORPUS.len());
    assert_eq!((ta, tb), (ta, ta));
    for r in 0..16 {
        assert_eq!(a_cpu.regs.r(r), b_cpu.regs.r(r), "r{r}");
    }
    assert_eq!(a_cpu.regs.cpsr(), b_cpu.regs.cpsr());
    for addr in (0x0200_0000..0x0200_0028).step_by(4) {
        assert_eq!(a_bus.read32(addr), b_bus.read32(addr), "cell {addr:#010X}");
    }
}

/// LDR pc, [pc]: R15 load diverts; retire flushes at target + lead.
#[test]
fn arm_ldr_pc_matches_legacy_and_flushes() {
    fn setup() -> (GbaCpu, GbaMemoryBus) {
        let mut cpu = GbaCpu::post_bios();
        let mut bus = GbaMemoryBus::new();
        bus.write32(0x0300_0000, 0xE59F_F000); // ldr pc, [pc]
        bus.write32(0x0300_0008, 0x0300_0010);
        bus.write32(0x0300_0010, 0xE3A0_7007); // mov r7, #7 (target)
        cpu.regs.set_pc(0x0300_0000);
        fill_pipeline(&mut cpu.regs, &mut bus, &mut cpu.pipeline);
        bus.take_access_wait_cycles();
        (cpu, bus)
    }
    let (mut a_cpu, mut a_bus) = setup();
    let ta = a_cpu.step_legacy(&mut a_bus);
    let (mut b_cpu, mut b_bus) = setup();
    let mut queue = std::collections::VecDeque::new();
    let mut acc = 0i64;
    loop {
        acc += step_op(
            &mut b_cpu.regs,
            &mut b_bus,
            &mut b_cpu.pipeline,
            &mut queue,
            false,
        )
        .expect("ldr-pc must be covered");
        if queue.is_empty() {
            break;
        }
    }
    let tb = acc.max(1) as u32;
    assert_eq!((ta, tb), (ta, ta));
    assert_eq!(b_cpu.regs.pc(), 0x0300_0018); // target + ARM lead
    assert_eq!(b_cpu.regs.pc(), a_cpu.regs.pc());
}

#[test]
fn arm_singlerest_tick_parity() {
    let (at, bt, av, bv) = tick_parity(
        &ARM_SINGLEREST_CORPUS,
        false,
        0x4014,
        0x0300_0000,
        &ARM_SINGLEREST_REGS,
        ARM_SINGLEREST_CORPUS.len(),
    );
    assert_eq!((at, bt), (at, at), "ticks diverge");
    assert_eq!((av, bv), (av, av), "timer span diverges");
}

// Slice 13 corpus: ARM halfword remainder (signed imm/reg,
// LDRH reg-offset, STRH reg-offset, LDRSH [pc]). Cell A holds
// 0x80FF so signedness is observable.
const ARM_HWREST_CORPUS: [u32; 9] = [
    0xE1D1_00F0, // ldrsh r0, [r1] (imm)
    0xE1D1_00D0, // ldrsb r0, [r1] (imm)
    0xE191_00B2, // ldrh r0, [r1, r2] (reg)
    0xE191_00F2, // ldrsh r0, [r1, r2] (reg)
    0xE191_00D2, // ldrsb r0, [r1, r2] (reg)
    0xE181_00B2, // strh r0, [r1, r2] (reg)
    0xE1DF_00F0, // ldrsh r0, [pc] (R15 base)
    0xE3A0_7007, // mov r7, #7
    0x0000_80FF, // pool halfword (never executed)
];
const ARM_HWREST_MEM: [(u32, u8, u32); 1] = [(0x0200_0000, 4, 0x0000_80FF)];
const ARM_HWREST_REGS: [(usize, u32); 2] = [(1, 0x0200_0000), (2, 0)];

#[test]
fn micro_op_arm_hwrest_matches_legacy() {
    for waitcnt in [0x0000u16, 0x0010, 0x4000, 0x4010, 0x4014] {
        let (ta, tb, regs_equal, follow) = differential(
            &ARM_HWREST_CORPUS,
            false,
            waitcnt,
            8,
            0xE1DD_20B0,
            0x0300_0000,
            None,
            &ARM_HWREST_MEM,
            &ARM_HWREST_REGS,
        );
        assert_eq!((ta, tb), (ta, ta), "iwram waitcnt={waitcnt:#06x}");
        assert!(regs_equal, "iwram waitcnt={waitcnt:#06x}");
        assert!(follow, "iwram waitcnt={waitcnt:#06x}");
    }
    for waitcnt in [0x0000u16, 0x4010, 0x4014] {
        let (ta, tb, regs_equal, follow) = differential(
            &ARM_HWREST_CORPUS,
            false,
            waitcnt,
            8,
            0xE1DD_20B0,
            0x0800_0100,
            Some(rom_cart()),
            &ARM_HWREST_MEM,
            &ARM_HWREST_REGS,
        );
        assert_eq!((ta, tb), (ta, ta), "rom waitcnt={waitcnt:#06x}");
        assert!(regs_equal, "rom waitcnt={waitcnt:#06x}");
        assert!(follow, "rom waitcnt={waitcnt:#06x}");
    }
}

#[test]
fn arm_hwrest_shapes_and_gates() {
    let mut regs = CpuRegisters::post_bios();
    regs.set_r(1, 0x0200_0000);
    regs.set_r(2, 4);
    // Signed/reg-offset forms expand like the unsigned-imm ones.
    for instr in [
        0xE1D1_00F0,
        0xE1D1_00D0,
        0xE191_00B2,
        0xE191_00F2,
        0xE1DF_00F0,
    ] {
        let ops = expand_arm(instr, &regs).expect("halfword form expands");
        assert_eq!(ops.len(), 3, "{instr:#010X}");
    }
    let ops = expand_arm(0xE181_00B2, &regs).expect("strh-reg expands");
    assert_eq!(ops.len(), 2);
    // Multiply/SWP keep the decoder-first routing (legacy here).
    assert!(expand_arm_single(0xE000_0090, &regs).is_none());
    assert!(expand_arm_single(0xE102_0091, &regs).is_none());
}

#[test]
fn arm_hwrest_memory_matches_legacy() {
    fn setup() -> (GbaCpu, GbaMemoryBus) {
        let mut cpu = GbaCpu::post_bios();
        let mut bus = GbaMemoryBus::new();
        for (i, w) in ARM_HWREST_CORPUS.iter().enumerate() {
            bus.write32(0x0300_0000 + (i as u32) * 4, *w);
        }
        for (addr, width, val) in ARM_HWREST_MEM {
            match width {
                4 => bus.write32(addr, val),
                2 => bus.write16(addr, (val & 0xFFFF) as u16),
                _ => bus.write8(addr, (val & 0xFF) as u8),
            }
        }
        for (r, v) in ARM_HWREST_REGS {
            cpu.regs.set_r(r, v);
        }
        cpu.regs.set_pc(0x0300_0000);
        fill_pipeline(&mut cpu.regs, &mut bus, &mut cpu.pipeline);
        bus.take_access_wait_cycles();
        (cpu, bus)
    }
    fn drain_micro(cpu: &mut GbaCpu, bus: &mut GbaMemoryBus, steps: usize) -> u32 {
        let mut total = 0u32;
        let mut queue = std::collections::VecDeque::new();
        for _ in 0..steps {
            let mut acc = 0i64;
            loop {
                acc += step_op(&mut cpu.regs, bus, &mut cpu.pipeline, &mut queue, false)
                    .expect("corpus must be covered");
                if queue.is_empty() {
                    break;
                }
            }
            total += acc.max(1) as u32;
        }
        total
    }
    let (mut a_cpu, mut a_bus) = setup();
    let (mut b_cpu, mut b_bus) = setup();
    let mut ta = 0u32;
    for _ in 0..8 {
        ta += a_cpu.step_legacy(&mut a_bus);
    }
    let tb = drain_micro(&mut b_cpu, &mut b_bus, 8);
    assert_eq!((ta, tb), (ta, ta));
    for r in 0..16 {
        assert_eq!(a_cpu.regs.r(r), b_cpu.regs.r(r), "r{r}");
    }
    assert_eq!(a_cpu.regs.cpsr(), b_cpu.regs.cpsr());
    assert_eq!(a_bus.read32(0x0200_0000), b_bus.read32(0x0200_0000));
}

/// ARM LDRSH at an odd BIOS address issues a guarded byte read,
/// never the Thumb odd-address quirk (guarded halfword + merge).
/// The byte is lane-selected from the 32-bit latch (0x56 of
/// 0x12345678 below), so this pins the bus call itself, not just
/// the sign math. The mgba-suite memory cells pin the same
/// property on hardware ("BIOS load S16 (unaligned)" = 0x20).
#[test]
fn arm_ldrsh_odd_guarded_byte_read() {
    fn setup() -> (GbaCpu, GbaMemoryBus) {
        let mut cpu = GbaCpu::post_bios();
        let mut bus = GbaMemoryBus::new();
        bus.write32(0x0300_0000, 0xE1D1_00F0); // ldrsh r0, [r1]
        bus.set_bios_prefetch(0x1234_5678);
        cpu.regs.set_r(1, 0x0000_0001); // odd BIOS address
        cpu.regs.set_pc(0x0300_0000);
        fill_pipeline(&mut cpu.regs, &mut bus, &mut cpu.pipeline);
        bus.take_access_wait_cycles();
        (cpu, bus)
    }
    let (mut a_cpu, mut a_bus) = setup();
    let ta = a_cpu.step_legacy(&mut a_bus);
    // Guarded byte read: address-lane latch byte, sign-extended.
    assert_eq!(a_cpu.regs.r(0), 0x56);
    let (mut b_cpu, mut b_bus) = setup();
    let mut queue = std::collections::VecDeque::new();
    let mut acc = 0i64;
    loop {
        acc += step_op(
            &mut b_cpu.regs,
            &mut b_bus,
            &mut b_cpu.pipeline,
            &mut queue,
            false,
        )
        .expect("ldrsh must be covered");
        if queue.is_empty() {
            break;
        }
    }
    let tb = acc.max(1) as u32;
    assert_eq!((ta, tb), (ta, ta));
    assert_eq!(b_cpu.regs.r(0), 0x56);
}

#[test]
fn arm_hwrest_tick_parity() {
    let (at, bt, av, bv) = tick_parity(
        &ARM_HWREST_CORPUS,
        false,
        0x4014,
        0x0300_0000,
        &ARM_HWREST_REGS,
        8,
    );
    assert_eq!((at, bt), (at, at), "ticks diverge");
    assert_eq!((av, bv), (av, av), "timer span diverges");
}

// Slice 14 corpus: ARM LDM/STM S-bit forms (run in SYS, where
// the user bank aliases the current one) plus a plain STM with PC
// (stored-base quirk + instruction+12 value).
const ARM_SBIT_CORPUS: [u32; 4] = [
    0xE965_0006, // stmdb r5!, {r1,r2}^
    0xE8F5_4018, // ldmia r5!, {r3,r4}^
    0xE8A6_8042, // stmia r6!, {r1,r6,r15}
    0xE3A0_7007, // mov r7, #7
];
const ARM_SBIT_REGS: [(usize, u32); 5] = [
    (1, 0x1111_1111),
    (2, 0x2222_2222),
    (5, 0x0200_0008),
    (6, 0x0200_0100),
    (7, 0),
];

#[test]
fn micro_op_arm_sbit_matches_legacy() {
    for waitcnt in [0x0000u16, 0x0010, 0x4000, 0x4010, 0x4014] {
        let (ta, tb, regs_equal, follow) = differential(
            &ARM_SBIT_CORPUS,
            false,
            waitcnt,
            ARM_SBIT_CORPUS.len(),
            0xE1DD_20B0,
            0x0300_0000,
            None,
            &[],
            &ARM_SBIT_REGS,
        );
        assert_eq!((ta, tb), (ta, ta), "iwram waitcnt={waitcnt:#06x}");
        assert!(regs_equal, "iwram waitcnt={waitcnt:#06x}");
        assert!(follow, "iwram waitcnt={waitcnt:#06x}");
    }
    for waitcnt in [0x0000u16, 0x4010, 0x4014] {
        let (ta, tb, regs_equal, follow) = differential(
            &ARM_SBIT_CORPUS,
            false,
            waitcnt,
            ARM_SBIT_CORPUS.len(),
            0xE1DD_20B0,
            0x0800_0100,
            Some(rom_cart()),
            &[],
            &ARM_SBIT_REGS,
        );
        assert_eq!((ta, tb), (ta, ta), "rom waitcnt={waitcnt:#06x}");
        assert!(regs_equal, "rom waitcnt={waitcnt:#06x}");
        assert!(follow, "rom waitcnt={waitcnt:#06x}");
    }
}

/// S-bit block memory effects in SYS: roundtrip words plus the
/// STM-PC value slot must agree.
#[test]
fn arm_sbit_memory_matches_legacy() {
    fn setup() -> (GbaCpu, GbaMemoryBus) {
        let mut cpu = GbaCpu::post_bios();
        let mut bus = GbaMemoryBus::new();
        for (i, w) in ARM_SBIT_CORPUS.iter().enumerate() {
            bus.write32(0x0300_0000 + (i as u32) * 4, *w);
        }
        for (r, v) in ARM_SBIT_REGS {
            cpu.regs.set_r(r, v);
        }
        cpu.regs.set_pc(0x0300_0000);
        fill_pipeline(&mut cpu.regs, &mut bus, &mut cpu.pipeline);
        bus.take_access_wait_cycles();
        (cpu, bus)
    }
    fn drain_micro(cpu: &mut GbaCpu, bus: &mut GbaMemoryBus, steps: usize) -> u32 {
        let mut total = 0u32;
        let mut queue = std::collections::VecDeque::new();
        for _ in 0..steps {
            let mut acc = 0i64;
            loop {
                acc += step_op(&mut cpu.regs, bus, &mut cpu.pipeline, &mut queue, false)
                    .expect("corpus must be covered");
                if queue.is_empty() {
                    break;
                }
            }
            total += acc.max(1) as u32;
        }
        total
    }
    let (mut a_cpu, mut a_bus) = setup();
    let (mut b_cpu, mut b_bus) = setup();
    let mut ta = 0u32;
    for _ in 0..ARM_SBIT_CORPUS.len() {
        ta += a_cpu.step_legacy(&mut a_bus);
    }
    let tb = drain_micro(&mut b_cpu, &mut b_bus, ARM_SBIT_CORPUS.len());
    assert_eq!((ta, tb), (ta, ta));
    for r in 0..16 {
        assert_eq!(a_cpu.regs.r(r), b_cpu.regs.r(r), "r{r}");
    }
    for addr in [
        0x0200_0000,
        0x0200_0004,
        0x0200_0100,
        0x0200_0104,
        0x0200_0108,
    ] {
        assert_eq!(a_bus.read32(addr), b_bus.read32(addr), "cell {addr:#010X}");
    }
}

/// User-bank transfer in IRQ mode: STM^ stores the user SP (not
/// the banked IRQ SP) and LDM^ loads the user LR.
#[test]
fn arm_block_s_user_bank_matches_legacy() {
    fn setup() -> (GbaCpu, GbaMemoryBus) {
        let mut cpu = GbaCpu::post_bios();
        let mut bus = GbaMemoryBus::new();
        bus.write32(0x0300_0000, 0xE8C5_2000); // stmia r5, {r13}^
        bus.write32(0x0300_0004, 0xE8D5_4000); // ldmia r5, {r14}^
        cpu.regs.enter_exception(0x12, 0x18, 0x0800_0000, true);
        cpu.regs.set_r(13, 0x0300_7000); // IRQ stack (banked)
        cpu.regs.set_r(5, 0x0200_0000);
        cpu.regs.set_pc(0x0300_0000);
        fill_pipeline(&mut cpu.regs, &mut bus, &mut cpu.pipeline);
        bus.take_access_wait_cycles();
        (cpu, bus)
    }
    fn drain_micro(cpu: &mut GbaCpu, bus: &mut GbaMemoryBus, steps: usize) -> u32 {
        let mut total = 0u32;
        let mut queue = std::collections::VecDeque::new();
        for _ in 0..steps {
            let mut acc = 0i64;
            loop {
                acc += step_op(&mut cpu.regs, bus, &mut cpu.pipeline, &mut queue, false)
                    .expect("corpus must be covered");
                if queue.is_empty() {
                    break;
                }
            }
            total += acc.max(1) as u32;
        }
        total
    }
    let (mut a_cpu, mut a_bus) = setup();
    let mut ta = 0u32;
    for _ in 0..2 {
        ta += a_cpu.step_legacy(&mut a_bus);
    }
    // The user SP (not the IRQ SP) hits memory.
    assert_eq!(a_bus.read32(0x0200_0000), 0x0300_7F00);
    let (mut b_cpu, mut b_bus) = setup();
    let tb = drain_micro(&mut b_cpu, &mut b_bus, 2);
    assert_eq!((ta, tb), (ta, ta));
    assert_eq!(b_bus.read32(0x0200_0000), 0x0300_7F00);
    for r in 0..16 {
        assert_eq!(a_cpu.regs.r(r), b_cpu.regs.r(r), "r{r}");
    }
    // User LR received the cell (bank-to-bank through memory).
    assert_eq!(a_cpu.regs.user_r(14), 0x0300_7F00);
    assert_eq!(b_cpu.regs.user_r(14), 0x0300_7F00);
}

/// LDM^ with PC in IRQ mode: CPSR restores from SPSR (back to SYS)
/// and PC loads; retire refills at the target.
#[test]
fn arm_ldm_s_pc_restore_matches_legacy() {
    fn setup() -> (GbaCpu, GbaMemoryBus) {
        let mut cpu = GbaCpu::post_bios();
        let mut bus = GbaMemoryBus::new();
        bus.write32(0x0300_0000, 0xE8D5_8001); // ldmia r5, {r0,pc}^
        bus.write32(0x0200_0000, 0);
        bus.write32(0x0200_0004, 0x0300_0010);
        bus.write32(0x0300_0010, 0xE3A0_7007); // mov r7, #7 (target)
        cpu.regs.enter_exception(0x12, 0x18, 0x0800_0000, true);
        // spsr_irq already holds pre-exception SYS cpsr (0x1F).
        cpu.regs.set_r(5, 0x0200_0000);
        cpu.regs.set_pc(0x0300_0000);
        fill_pipeline(&mut cpu.regs, &mut bus, &mut cpu.pipeline);
        bus.take_access_wait_cycles();
        (cpu, bus)
    }
    let (mut a_cpu, mut a_bus) = setup();
    let ta = a_cpu.step_legacy(&mut a_bus);
    assert_eq!(a_cpu.regs.cpsr_mode(), 0x1F);
    assert_eq!(a_cpu.regs.pc(), 0x0300_0018);
    let (mut b_cpu, mut b_bus) = setup();
    let mut queue = std::collections::VecDeque::new();
    let mut acc = 0i64;
    loop {
        acc += step_op(
            &mut b_cpu.regs,
            &mut b_bus,
            &mut b_cpu.pipeline,
            &mut queue,
            false,
        )
        .expect("ldm^-pc must be covered");
        if queue.is_empty() {
            break;
        }
    }
    let tb = acc.max(1) as u32;
    assert_eq!((ta, tb), (ta, ta));
    assert_eq!(b_cpu.regs.cpsr_mode(), 0x1F);
    assert_eq!(b_cpu.regs.pc(), 0x0300_0018);
    assert_eq!(b_cpu.regs.r(0), 0);
    assert_eq!(b_cpu.regs.r(0), a_cpu.regs.r(0));
}

#[test]
fn arm_sbit_tick_parity() {
    let (at, bt, av, bv) = tick_parity(
        &ARM_SBIT_CORPUS,
        false,
        0x4014,
        0x0300_0000,
        &ARM_SBIT_REGS,
        ARM_SBIT_CORPUS.len(),
    );
    assert_eq!((at, bt), (at, at), "ticks diverge");
    assert_eq!((av, bv), (av, av), "timer span diverges");
}

// Slice 15 corpus: ARM SWP/SWPB roundtrip through an EWRAM cell.
const ARM_SWP_CORPUS: [u32; 3] = [
    0xE102_0091, // swp r0, r1, [r2]
    0xE142_0091, // swpb r0, r1, [r2]
    0xE3A0_7007, // mov r7, #7
];

#[test]
fn micro_op_arm_swp_matches_legacy() {
    for waitcnt in [0x0000u16, 0x0010, 0x4000, 0x4010, 0x4014] {
        let (ta, tb, regs_equal, follow) = differential(
            &ARM_SWP_CORPUS,
            false,
            waitcnt,
            ARM_SWP_CORPUS.len(),
            0xE1DD_20B0,
            0x0300_0000,
            None,
            &[(0x0200_0000, 4, 0x5555_5555)],
            &[(1, 0xAAAA_AAAA), (2, 0x0200_0000)],
        );
        assert_eq!((ta, tb), (ta, ta), "iwram waitcnt={waitcnt:#06x}");
        assert!(regs_equal, "iwram waitcnt={waitcnt:#06x}");
        assert!(follow, "iwram waitcnt={waitcnt:#06x}");
    }
    for waitcnt in [0x0000u16, 0x4010, 0x4014] {
        let (ta, tb, regs_equal, follow) = differential(
            &ARM_SWP_CORPUS,
            false,
            waitcnt,
            ARM_SWP_CORPUS.len(),
            0xE1DD_20B0,
            0x0800_0100,
            Some(rom_cart()),
            &[(0x0200_0000, 4, 0x5555_5555)],
            &[(1, 0xAAAA_AAAA), (2, 0x0200_0000)],
        );
        assert_eq!((ta, tb), (ta, ta), "rom waitcnt={waitcnt:#06x}");
        assert!(regs_equal, "rom waitcnt={waitcnt:#06x}");
        assert!(follow, "rom waitcnt={waitcnt:#06x}");
    }
}

#[test]
fn arm_swp_shapes() {
    let mut regs = CpuRegisters::post_bios();
    regs.set_r(2, 0x0200_0000);
    // Atomic commit + 3 trailing = 4.
    let ops = expand_arm(0xE102_0091, &regs).expect("swp expands");
    assert_eq!(ops.len(), 4);
    let ops = expand_arm(0xE142_0091, &regs).expect("swpb expands");
    assert_eq!(ops.len(), 4);
    // The single-transfer branch must not claim the SWP mask.
    assert!(expand_arm_single(0xE102_0091, &regs).is_none());
}

#[test]
fn arm_swp_memory_matches_legacy() {
    fn setup() -> (GbaCpu, GbaMemoryBus) {
        let mut cpu = GbaCpu::post_bios();
        let mut bus = GbaMemoryBus::new();
        for (i, w) in ARM_SWP_CORPUS.iter().enumerate() {
            bus.write32(0x0300_0000 + (i as u32) * 4, *w);
        }
        bus.write32(0x0200_0000, 0x5555_5555);
        cpu.regs.set_r(1, 0xAAAA_AAAA);
        cpu.regs.set_r(2, 0x0200_0000);
        cpu.regs.set_pc(0x0300_0000);
        fill_pipeline(&mut cpu.regs, &mut bus, &mut cpu.pipeline);
        bus.take_access_wait_cycles();
        (cpu, bus)
    }
    fn drain_micro(cpu: &mut GbaCpu, bus: &mut GbaMemoryBus, steps: usize) -> u32 {
        let mut total = 0u32;
        let mut queue = std::collections::VecDeque::new();
        for _ in 0..steps {
            let mut acc = 0i64;
            loop {
                acc += step_op(&mut cpu.regs, bus, &mut cpu.pipeline, &mut queue, false)
                    .expect("corpus must be covered");
                if queue.is_empty() {
                    break;
                }
            }
            total += acc.max(1) as u32;
        }
        total
    }
    let (mut a_cpu, mut a_bus) = setup();
    let (mut b_cpu, mut b_bus) = setup();
    let mut ta = 0u32;
    for _ in 0..ARM_SWP_CORPUS.len() {
        ta += a_cpu.step_legacy(&mut a_bus);
    }
    let tb = drain_micro(&mut b_cpu, &mut b_bus, ARM_SWP_CORPUS.len());
    assert_eq!((ta, tb), (ta, ta));
    for r in 0..16 {
        assert_eq!(a_cpu.regs.r(r), b_cpu.regs.r(r), "r{r}");
    }
    // SWP exchanged, SWPB rewrote the low byte with itself.
    assert_eq!(a_bus.read32(0x0200_0000), 0xAAAA_AAAA);
    assert_eq!(b_bus.read32(0x0200_0000), 0xAAAA_AAAA);
}

#[test]
fn arm_swp_tick_parity() {
    let (at, bt, av, bv) = tick_parity(
        &ARM_SWP_CORPUS,
        false,
        0x4014,
        0x0300_0000,
        &[(1, 0xAAAA_AAAA), (2, 0x0200_0000)],
        ARM_SWP_CORPUS.len(),
    );
    assert_eq!((at, bt), (at, at), "ticks diverge");
    assert_eq!((av, bv), (av, av), "timer span diverges");
}

// Slice 16 corpus: ARM MRS/MSR + BX over a relative target
// (ADD pc keeps the corpus position-independent for the ROM
// variant). MSR sets N via r0; TST observes it; BX skips two.
const ARM_PSRBX_CORPUS: [u32; 10] = [
    0xE10F_1000, // mrs r1, cpsr
    0xE129_F000, // msr cpsr_f, r0
    0xE111_0000, // tst r1, r0
    0xE28F_300C, // add r3, pc, #12
    0xE12F_FF13, // bx r3
    0xE3A0_4005, // mov r4, #5 (skipped)
    0xE3A0_5006, // mov r5, #6 (skipped)
    0xE3A0_2009, // mov r2, #9 (skipped)
    0xE3A0_6007, // mov r6, #7 (BX landing)
    0xE3A0_7008, // mov r7, #8
];
const ARM_PSRBX_REGS: [(usize, u32); 2] = [(0, 0x8000_0000), (3, 0)];

#[test]
fn micro_op_arm_psrbx_matches_legacy() {
    for waitcnt in [0x0000u16, 0x0010, 0x4000, 0x4010, 0x4014] {
        let (ta, tb, regs_equal, follow) = differential(
            &ARM_PSRBX_CORPUS,
            false,
            waitcnt,
            7,
            0xE1DD_20B0,
            0x0300_0000,
            None,
            &[],
            &ARM_PSRBX_REGS,
        );
        assert_eq!((ta, tb), (ta, ta), "iwram waitcnt={waitcnt:#06x}");
        assert!(regs_equal, "iwram waitcnt={waitcnt:#06x}");
        assert!(follow, "iwram waitcnt={waitcnt:#06x}");
    }
    for waitcnt in [0x0000u16, 0x4010, 0x4014] {
        let (ta, tb, regs_equal, follow) = differential(
            &ARM_PSRBX_CORPUS,
            false,
            waitcnt,
            7,
            0xE1DD_20B0,
            0x0800_0100,
            Some(rom_cart()),
            &[],
            &ARM_PSRBX_REGS,
        );
        assert_eq!((ta, tb), (ta, ta), "rom waitcnt={waitcnt:#06x}");
        assert!(regs_equal, "rom waitcnt={waitcnt:#06x}");
        assert!(follow, "rom waitcnt={waitcnt:#06x}");
    }
}

#[test]
fn arm_psrbx_shapes_and_gates() {
    let regs = CpuRegisters::post_bios();
    // PSR forms: single commit op.
    for instr in [0xE10F_1000, 0xE129_F000, 0xE32B_F000] {
        let ops = expand_arm(instr, &regs).expect("psr expands");
        assert_eq!(ops.len(), 1, "{instr:#010X}");
    }
    // ARM BX: refill pair + commit.
    let ops = expand_arm(0xE12F_FF13, &regs).expect("arm bx expands");
    assert_eq!(ops.len(), 3);
    // DP-imm with Rn==PC reads the execute-stage PC (bus-free).
    let ops = expand_arm(0xE28F_300C, &regs).expect("add-pc expands");
    assert_eq!(ops.len(), 1);
}

/// ARM BX to Thumb: mode switches at execution; retire refills
/// with halfword width.
#[test]
fn arm_bx_to_thumb_matches_legacy() {
    fn setup() -> (GbaCpu, GbaMemoryBus) {
        let mut cpu = GbaCpu::post_bios();
        let mut bus = GbaMemoryBus::new();
        bus.write32(0x0200_0000, 0xE12F_FF10); // bx r0
        bus.write16(0x0300_0040, 0x2707); // mov r7, #7 (Thumb target)
        cpu.regs.set_r(0, 0x0300_0041); // bit0 set: Thumb
        cpu.regs.set_pc(0x0200_0000);
        fill_pipeline(&mut cpu.regs, &mut bus, &mut cpu.pipeline);
        bus.take_access_wait_cycles();
        (cpu, bus)
    }
    let (mut a_cpu, mut a_bus) = setup();
    let ta = a_cpu.step_legacy(&mut a_bus);
    let (mut b_cpu, mut b_bus) = setup();
    let mut queue = std::collections::VecDeque::new();
    let mut acc = 0i64;
    loop {
        acc += step_op(
            &mut b_cpu.regs,
            &mut b_bus,
            &mut b_cpu.pipeline,
            &mut queue,
            false,
        )
        .expect("arm bx must be covered");
        if queue.is_empty() {
            break;
        }
    }
    let tb = acc.max(1) as u32;
    assert_eq!((ta, tb), (ta, ta));
    assert_eq!(b_cpu.regs.pc(), 0x0300_0044); // target + Thumb lead
    assert_eq!(b_cpu.regs.pc(), a_cpu.regs.pc());
    assert!(b_cpu.regs.cpsr_t());
    assert_eq!(b_cpu.regs.cpsr_t(), a_cpu.regs.cpsr_t());
}

#[test]
fn arm_psrbx_tick_parity() {
    let (at, bt, av, bv) = tick_parity(
        &ARM_PSRBX_CORPUS,
        false,
        0x4014,
        0x0300_0000,
        &ARM_PSRBX_REGS,
        7,
    );
    assert_eq!((at, bt), (at, at), "ticks diverge");
    assert_eq!((av, bv), (av, av), "timer span diverges");
}

// Slice 17 corpus: ARM DP-imm full set (logical, reverse and
// carry arithmetic, flag-only forms, rotated S=0). r1 carries
// 0xFF00FF00; the SUBS seeds C=1 for the SBC/RSC chain.
const ARM_ALUFULL_CORPUS: [u32; 16] = [
    0xE201_00FF, // and r0, r1, #0xFF
    0xE221_00FF, // eor r0, r1, #0xFF
    0xE251_0001, // subs r0, r1, #1 (C=1)
    0xE2C1_0000, // sbc r0, r1, #0
    0xE2E1_0000, // rsc r0, r1, #0
    0xE261_0001, // rsb r0, r1, #1
    0xE2A1_0001, // adc r0, r1, #1
    0xE311_00FF, // tst r1, #0xFF
    0xE111_0000, // teq r1, r0 (S=0: flags untouched)
    0xE371_0001, // cmn r1, #1
    0xE381_0001, // orr r0, r1, #1
    0xE3C1_00FF, // bic r0, r1, #0xFF
    0xE3E0_0000, // mvn r0, #0
    0xE201_04FF, // and r0, r1, #0xFF000000 (rotated, S=0)
    0xE3A0_3005, // mov r3, #5
    0xE3A0_7007, // mov r7, #7
];

#[test]
fn micro_op_arm_alufull_matches_legacy() {
    for waitcnt in [0x0000u16, 0x0010, 0x4000, 0x4010, 0x4014] {
        let (ta, tb, regs_equal, follow) = differential(
            &ARM_ALUFULL_CORPUS,
            false,
            waitcnt,
            ARM_ALUFULL_CORPUS.len(),
            0xE1DD_20B0,
            0x0300_0000,
            None,
            &[],
            &[(1, 0xFF00_FF00)],
        );
        assert_eq!((ta, tb), (ta, ta), "iwram waitcnt={waitcnt:#06x}");
        assert!(regs_equal, "iwram waitcnt={waitcnt:#06x}");
        assert!(follow, "iwram waitcnt={waitcnt:#06x}");
    }
    for waitcnt in [0x0000u16, 0x4010, 0x4014] {
        let (ta, tb, regs_equal, follow) = differential(
            &ARM_ALUFULL_CORPUS,
            false,
            waitcnt,
            ARM_ALUFULL_CORPUS.len(),
            0xE1DD_20B0,
            0x0800_0100,
            Some(rom_cart()),
            &[],
            &[(1, 0xFF00_FF00)],
        );
        assert_eq!((ta, tb), (ta, ta), "rom waitcnt={waitcnt:#06x}");
        assert!(regs_equal, "rom waitcnt={waitcnt:#06x}");
        assert!(follow, "rom waitcnt={waitcnt:#06x}");
    }
}

#[test]
fn arm_alufull_tick_parity() {
    let (at, bt, av, bv) = tick_parity(
        &ARM_ALUFULL_CORPUS,
        false,
        0x4014,
        0x0300_0000,
        &[(1, 0xFF00_FF00)],
        ARM_ALUFULL_CORPUS.len(),
    );
    assert_eq!((at, bt), (at, at), "ticks diverge");
    assert_eq!((av, bv), (av, av), "timer span diverges");
}

// Slice 18 corpus: S with rotated immediates (shifter carry).
const ARM_SROT_CORPUS: [u32; 5] = [
    0xE211_04FF, // ands r0, r1, #0xFF000000 (C=1)
    0xE291_04FF, // adds r0, r1, #0xFF000000
    0xE3B0_04FF, // movs r0, #0xFF000000 (N=1, C=1)
    0xE3A0_3005, // mov r3, #5
    0xE3A0_7007, // mov r7, #7
];

#[test]
fn micro_op_arm_srot_matches_legacy() {
    for waitcnt in [0x0000u16, 0x0010, 0x4000, 0x4010, 0x4014] {
        let (ta, tb, regs_equal, follow) = differential(
            &ARM_SROT_CORPUS,
            false,
            waitcnt,
            ARM_SROT_CORPUS.len(),
            0xE1DD_20B0,
            0x0300_0000,
            None,
            &[],
            &[(1, 1)],
        );
        assert_eq!((ta, tb), (ta, ta), "iwram waitcnt={waitcnt:#06x}");
        assert!(regs_equal, "iwram waitcnt={waitcnt:#06x}");
        assert!(follow, "iwram waitcnt={waitcnt:#06x}");
    }
    for waitcnt in [0x0000u16, 0x4010, 0x4014] {
        let (ta, tb, regs_equal, follow) = differential(
            &ARM_SROT_CORPUS,
            false,
            waitcnt,
            ARM_SROT_CORPUS.len(),
            0xE1DD_20B0,
            0x0800_0100,
            Some(rom_cart()),
            &[],
            &[(1, 1)],
        );
        assert_eq!((ta, tb), (ta, ta), "rom waitcnt={waitcnt:#06x}");
        assert!(regs_equal, "rom waitcnt={waitcnt:#06x}");
        assert!(follow, "rom waitcnt={waitcnt:#06x}");
    }
}

/// SUBS pc, lr, #4 in IRQ mode (immediate form): CPSR restores
/// from SPSR (to SVC here) and PC loads; retire refills.
#[test]
fn arm_subs_pc_restore_matches_legacy() {
    fn setup() -> (GbaCpu, GbaMemoryBus) {
        let mut cpu = GbaCpu::post_bios();
        let mut bus = GbaMemoryBus::new();
        bus.write32(0x0300_0000, 0xE25E_F004); // subs pc, lr, #4
        bus.write32(0x0300_0010, 0xE3A0_7007); // mov r7, #7 (target)
        cpu.regs.enter_exception(0x12, 0x18, 0x0800_0000, true);
        cpu.regs.set_spsr(0x13); // return to SVC on restore
        cpu.regs.set_lr(0x0300_0014);
        cpu.regs.set_pc(0x0300_0000);
        fill_pipeline(&mut cpu.regs, &mut bus, &mut cpu.pipeline);
        bus.take_access_wait_cycles();
        (cpu, bus)
    }
    let (mut a_cpu, mut a_bus) = setup();
    let ta = a_cpu.step_legacy(&mut a_bus);
    assert_eq!(a_cpu.regs.cpsr_mode(), 0x13);
    assert_eq!(a_cpu.regs.pc(), 0x0300_0018);
    let (mut b_cpu, mut b_bus) = setup();
    let mut queue = std::collections::VecDeque::new();
    let mut acc = 0i64;
    loop {
        acc += step_op(
            &mut b_cpu.regs,
            &mut b_bus,
            &mut b_cpu.pipeline,
            &mut queue,
            false,
        )
        .expect("subs-pc must be covered");
        if queue.is_empty() {
            break;
        }
    }
    let tb = acc.max(1) as u32;
    assert_eq!((ta, tb), (ta, ta));
    assert_eq!(b_cpu.regs.cpsr_mode(), 0x13);
    assert_eq!(b_cpu.regs.pc(), 0x0300_0018);
}

/// MOVS pc, lr in IRQ mode (register form, delegate path): same
/// restore + load contract through CommitDpReg.
#[test]
fn arm_movs_pc_restore_matches_legacy() {
    fn setup() -> (GbaCpu, GbaMemoryBus) {
        let mut cpu = GbaCpu::post_bios();
        let mut bus = GbaMemoryBus::new();
        bus.write32(0x0300_0000, 0xE1B0_F00E); // movs pc, lr
        bus.write32(0x0300_0010, 0xE3A0_7007); // mov r7, #7 (target)
        cpu.regs.enter_exception(0x12, 0x18, 0x0800_0000, true);
        cpu.regs.set_spsr(0x13); // return to SVC on restore
        cpu.regs.set_lr(0x0300_0010);
        cpu.regs.set_pc(0x0300_0000);
        fill_pipeline(&mut cpu.regs, &mut bus, &mut cpu.pipeline);
        bus.take_access_wait_cycles();
        (cpu, bus)
    }
    let (mut a_cpu, mut a_bus) = setup();
    let ta = a_cpu.step_legacy(&mut a_bus);
    assert_eq!(a_cpu.regs.cpsr_mode(), 0x13);
    assert_eq!(a_cpu.regs.pc(), 0x0300_0018);
    let (mut b_cpu, mut b_bus) = setup();
    let mut queue = std::collections::VecDeque::new();
    let mut acc = 0i64;
    loop {
        acc += step_op(
            &mut b_cpu.regs,
            &mut b_bus,
            &mut b_cpu.pipeline,
            &mut queue,
            false,
        )
        .expect("movs-pc must be covered");
        if queue.is_empty() {
            break;
        }
    }
    let tb = acc.max(1) as u32;
    assert_eq!((ta, tb), (ta, ta));
    assert_eq!(b_cpu.regs.cpsr_mode(), 0x13);
    assert_eq!(b_cpu.regs.pc(), 0x0300_0018);
}

/// SUBS pc, lr, #0 in SYS mode (no SPSR): PC writes and flags
/// update, no restore.
#[test]
fn arm_subs_pc_sys_matches_legacy() {
    fn setup() -> (GbaCpu, GbaMemoryBus) {
        let mut cpu = GbaCpu::post_bios();
        let mut bus = GbaMemoryBus::new();
        bus.write32(0x0300_0000, 0xE25E_F000); // subs pc, lr, #0
        bus.write32(0x0300_0010, 0xE3A0_7007); // mov r7, #7 (target)
        cpu.regs.set_lr(0x0300_0010);
        cpu.regs.set_pc(0x0300_0000);
        fill_pipeline(&mut cpu.regs, &mut bus, &mut cpu.pipeline);
        bus.take_access_wait_cycles();
        (cpu, bus)
    }
    let (mut a_cpu, mut a_bus) = setup();
    let ta = a_cpu.step_legacy(&mut a_bus);
    let (mut b_cpu, mut b_bus) = setup();
    let mut queue = std::collections::VecDeque::new();
    let mut acc = 0i64;
    loop {
        acc += step_op(
            &mut b_cpu.regs,
            &mut b_bus,
            &mut b_cpu.pipeline,
            &mut queue,
            false,
        )
        .expect("subs-pc must be covered");
        if queue.is_empty() {
            break;
        }
    }
    let tb = acc.max(1) as u32;
    assert_eq!((ta, tb), (ta, ta));
    assert_eq!(b_cpu.regs.pc(), 0x0300_0018);
    assert_eq!(b_cpu.regs.pc(), a_cpu.regs.pc());
    assert_eq!(b_cpu.regs.cpsr(), a_cpu.regs.cpsr());
}

/// Flag-only Rd==15 with S in IRQ mode: CPSR restores, PC advances
/// without a flush (legacy refill count, no latch).
#[test]
fn arm_tst_pc_restore_no_flush() {
    fn setup() -> (GbaCpu, GbaMemoryBus) {
        let mut cpu = GbaCpu::post_bios();
        let mut bus = GbaMemoryBus::new();
        bus.write32(0x0300_0000, 0xE111_F00E); // tst pc, lr (S=1)
        bus.write32(0x0300_0004, 0xE3A0_7007); // mov r7, #7
        cpu.regs.enter_exception(0x12, 0x18, 0x0800_0000, true);
        cpu.regs.set_spsr(0x1F); // restore SYS (ARM)
        cpu.regs.set_lr(0x0300_0100);
        cpu.regs.set_pc(0x0300_0000);
        fill_pipeline(&mut cpu.regs, &mut bus, &mut cpu.pipeline);
        bus.take_access_wait_cycles();
        (cpu, bus)
    }
    let (mut a_cpu, mut a_bus) = setup();
    let ta = a_cpu.step_legacy(&mut a_bus);
    assert_eq!(a_cpu.regs.cpsr_mode(), 0x1F);
    let (mut b_cpu, mut b_bus) = setup();
    let mut queue = std::collections::VecDeque::new();
    let mut acc = 0i64;
    loop {
        acc += step_op(
            &mut b_cpu.regs,
            &mut b_bus,
            &mut b_cpu.pipeline,
            &mut queue,
            false,
        )
        .expect("tst-pc must be covered");
        if queue.is_empty() {
            break;
        }
    }
    let tb = acc.max(1) as u32;
    assert_eq!((ta, tb), (ta, ta));
    assert_eq!(b_cpu.regs.cpsr_mode(), 0x1F);
    // No flush: sequential advance past the restored stream.
    assert_eq!(b_cpu.regs.pc(), a_cpu.regs.pc());
}

#[test]
fn arm_srot_tick_parity() {
    let (at, bt, av, bv) = tick_parity(
        &ARM_SROT_CORPUS,
        false,
        0x4014,
        0x0300_0000,
        &[(1, 1)],
        ARM_SROT_CORPUS.len(),
    );
    assert_eq!((at, bt), (at, at), "ticks diverge");
    assert_eq!((av, bv), (av, av), "timer span diverges");
}

/// Differential grid over S+rotated DP-immediate space: all 16
/// opcodes, rotation fields, low immediates and base registers.
/// Pins the shifter-carry snapshot against the live CPSR carry
/// (ADC/SBC/RSC read the latter even under rotation).
#[test]
fn arm_dpimm_srot_grid() {
    let mut bad = 0;
    for opcode in 0..16u32 {
        for rot in [1u32, 2, 4, 7, 15] {
            for imm8 in [0u32, 1, 0xFF, 0x80] {
                for rn in [0usize, 1] {
                    let instr = 0xE000_0000
                        | (opcode << 21)
                        | (1 << 20)
                        | ((rn as u32) << 16)
                        | (rot << 8)
                        | imm8
                        | (1 << 25);
                    let code = [instr, 0xE3A0_7007];
                    let (ta, tb, regs, follow) = differential(
                        &code,
                        false,
                        0x0000,
                        1,
                        0xE1DD_20B0,
                        0x0300_0000,
                        None,
                        &[],
                        &[(0, 0x1234_5678), (1, 0xFF00_FF00)],
                    );
                    if ta != tb || !regs || !follow {
                        eprintln!(
                            "DIVERGE op={:#X} rot={} imm={:#X} rn={}: ta={} tb={} regs={} follow={}",
                            opcode, rot, imm8, rn, ta, tb, regs, follow
                        );
                        bad += 1;
                        if bad > 8 {
                            panic!("too many divergences");
                        }
                    }
                }
            }
        }
    }
    assert_eq!(bad, 0);
}

/// Coverage manifest: every instruction class is either expanded
/// or intentionally legacy. Intentional-legacy (None): empty block
/// lists (quirk paths), UND/SWI (exceptions), coprocessor (UND on
/// GBA). Everything else in both ISAs must expand; add new classes
/// here when extending coverage.
#[test]
fn coverage_manifest() {
    let regs = CpuRegisters::post_bios();
    // Intentionally legacy ARM: empty lists, SWI, coprocessor UND.
    for instr in [
        0xE8A0_0000, // empty stmia
        0xE8F0_0000, // empty ldmia^
        0xEF00_0000, // swi
        0xEE00_0010, // coprocessor (UND)
        0xEC00_0000, // coprocessor (UND)
    ] {
        assert!(expand_arm(instr, &regs).is_none(), "{instr:#010X}");
    }
    // Intentionally legacy Thumb: empty lists, UND, SWI.
    let mut tregs = CpuRegisters::post_bios();
    tregs.set_cpsr(tregs.cpsr() | (1 << 5));
    for instr in [
        0xB400, // empty push
        0xBC00, // empty pop
        0xC000, // empty stmia
        0xC800, // empty ldmia
        0xDE00, // undefined
        0xDF00, // swi
    ] {
        assert!(expand_thumb(instr, &tregs).is_none(), "{instr:#06X}");
    }
    // Covered ARM representatives (one per class/form).
    let mut aregs = CpuRegisters::post_bios();
    aregs.set_r(0, 0x0200_0000);
    aregs.set_r(1, 0xFF00_FF00);
    aregs.set_r(2, 4);
    aregs.set_r(5, 0x0200_0008);
    for instr in [
        0xE3A0_0001, // dp-imm mov
        0xE211_04FF, // dp-imm S+rot
        0xE3A0_F005, // dp-imm Rd==15
        0xE081_0002, // dp-reg
        0xE1B0_0213, // dp-reg shift
        0xE10F_1000, // mrs
        0xE129_F000, // msr-reg
        0xE32B_F000, // msr-imm
        0xE12F_FF11, // bx
        0xE000_0291, // mul
        0xE022_0391, // mla
        0xE083_2190, // umull
        0xE102_0091, // swp
        0xE581_0004, // str imm
        0xE791_0002, // ldr reg-offset
        0xE59F_5000, // ldr literal (rn==pc)
        0xE59F_F000, // ldr pc (Rd==15)
        0xE1D1_00B0, // ldrh imm (S:H == 01)
        0xE1D1_00F0, // ldrsh imm (S:H == 11)
        0xE1D1_00D0, // ldrsb imm (S:H == 10)
        0xE191_00B2, // ldrh reg-offset
        0xE7C1_3004, // strb reg-offset
        0xE1D1_00F0, // ldrsh imm
        0xE191_00B2, // ldrh reg-offset
        0xE8A0_0006, // stmia
        0xE8B5_0018, // ldmia
        0xE8F5_4018, // ldmia^ (S bit)
        0xE890_8000, // ldmia pc
        0xEA00_0001, // b
        0xEB00_0001, // bl (link)
    ] {
        assert!(expand_arm(instr, &aregs).is_some(), "{instr:#010X}");
    }
    // Covered Thumb representatives (one per class/form).
    for instr in [
        0x0041, // lsl imm
        0x1881, // add reg
        0x2001, // mov imm
        0x4341, // mul
        0x4001, // and reg
        0x41C1, // ror reg (+1I)
        0x4400, // add hi-reg
        0x4487, // add pc (+2)
        0x4580, // cmp hi-reg
        0x4770, // bx
        0x4801, // ldr literal
        0x5088, // str reg-offset
        0x5E8F, // ldrsh reg-offset
        0x6088, // str imm-offset
        0x8090, // strh
        0x9010, // str sp-relative
        0x9D10, // ldr sp-relative
        0xA004, // add pc
        0xB00A, // add sp
        0xB40F, // push
        0xBCF0, // pop
        0xBD02, // pop pc
        0xC10F, // stmia
        0xCB18, // ldmia (base in list)
        0xD001, // cond branch
        0xE001, // b
        0xF000, // bl high
        0xF806, // bl low
    ] {
        assert!(expand_thumb(instr, &tregs).is_some(), "{instr:#06X}");
    }
}

#[test]
fn micro_op_rom_code_matches_legacy() {
    for waitcnt in [0x0000u16, 0x4010, 0x4014] {
        let (ta, tb, regs, follow) = differential(
            &ARM_ROM_CORPUS,
            false,
            waitcnt,
            9,
            0xE1DD_20B0,
            0x0800_0100,
            Some(rom_cart()),
            &[(0x0200_0008, 4, 0xAABB_CCDD)],
            &[(0, 0x1234_5678)],
        );
        assert_eq!((ta, tb), (ta, ta), "waitcnt={waitcnt:#06x}");
        assert!(regs, "waitcnt={waitcnt:#06x}");
        assert!(follow, "waitcnt={waitcnt:#06x}");
    }
}

#[test]
fn micro_op_flag_carryover() {
    // V set by an overflowing ADD must survive a following MOV
    // (logical class preserves V in both modes; the legacy Thumb
    // MOV-imm additionally forces N=0). Caught by differential:
    // legacy keeps V=1, a clobbering engine would read V=0.
    let (ta, tb, regs, follow) = differential(
        &[0x3001u32, 0x2102], // add r0, #1 (0x7FFFFFFF -> V=1); mov r1, #2
        true,
        0x0000,
        2,
        0x886A,
        0x0300_0000,
        None,
        &[],
        &[(0, 0x7FFF_FFFF)],
    );
    assert_eq!((ta, tb), (ta, ta));
    assert!(regs);
    assert!(follow);
}

/// Tick parity between legacy and micro drain rhythms, ticking the bus per elapsed tick with TM0 running.
/// Returns (legacy_ticks, micro_ticks, legacy_tm0, micro_tm0).
/// Divergence here shifts every timer-measured cell, pinning the system drain loop.
fn tick_parity(
    code: &[u32],
    thumb: bool,
    waitcnt: u16,
    code_base: u32,
    reg_init: &[(usize, u32)],
    steps: usize,
) -> (u32, u32, u32, u32) {
    fn setup(
        code: &[u32],
        thumb: bool,
        waitcnt: u16,
        code_base: u32,
        reg_init: &[(usize, u32)],
    ) -> (GbaCpu, GbaMemoryBus) {
        let mut cpu = GbaCpu::post_bios();
        let mut bus = GbaMemoryBus::new();
        bus.write16(0x04000204, waitcnt);
        let stride = if thumb { 2 } else { 4 };
        for (i, w) in code.iter().enumerate() {
            let addr = code_base + (i as u32) * stride as u32;
            if thumb {
                bus.write16(addr, (w & 0xFFFF) as u16);
            } else {
                bus.write32(addr, *w);
            }
        }
        for (r, v) in reg_init {
            cpu.regs.set_r(*r, *v);
        }
        if thumb {
            cpu.regs.set_cpsr(cpu.regs.cpsr() | (1 << 5));
        }
        cpu.regs.set_pc(code_base);
        fill_pipeline(&mut cpu.regs, &mut bus, &mut cpu.pipeline);
        bus.take_access_wait_cycles();
        // TM0 free-run /1 from 0 (the suite START shape, minus the
        // control write which the corpus itself performs if needed).
        bus.write32(0x0400_0100, 0x0080_0000);
        bus.take_access_wait_cycles();
        (cpu, bus)
    }
    // Legacy rhythm: whole step, then one tick per returned cycle.
    let (mut a_cpu, mut a_bus) = setup(code, thumb, waitcnt, code_base, reg_init);
    let mut a_ticks = 0u32;
    for _ in 0..steps {
        let c = a_cpu.step_legacy(&mut a_bus).max(1);
        for _ in 0..c {
            a_bus.tick();
            a_ticks += 1;
        }
    }
    let a_tm0 = a_bus.read16(0x0400_0100);
    // Micro rhythm: mirror of the system.rs drain loop, one tick at a
    // time until every corpus instruction has retired.
    let (mut b_cpu, mut b_bus) = setup(code, thumb, waitcnt, code_base, reg_init);
    let mut b_queue = std::collections::VecDeque::new();
    let mut b_ticks = 0u32;
    let mut retired = 0usize;
    while retired < steps {
        let mut acc = 0i64;
        loop {
            let c = step_op(
                &mut b_cpu.regs,
                &mut b_bus,
                &mut b_cpu.pipeline,
                &mut b_queue,
                thumb,
            )
            .expect("corpus must be covered");
            acc += c;
            if b_queue.is_empty() {
                retired += 1;
                break;
            }
            if acc >= 1 {
                break;
            }
        }
        let spend = acc.max(1) as u32;
        for _ in 0..spend {
            b_bus.tick();
            b_ticks += 1;
        }
    }
    let b_tm0 = b_bus.read16(0x0400_0100);
    (a_ticks, b_ticks, u32::from(a_tm0), u32::from(b_tm0))
}

#[test]
fn micro_op_tick_parity_timer_span() {
    // Calibration shape: TM0 start already running (setup), one
    // payload read, one control write — the measured span must match
    // between rhythms (IWRAM code, Thumb).
    let code = [
        0x9802u32, // ldr r0, [sp, #8] (sp data, covered)
        0x9003,    // str r0, [sp, #12]
    ];
    let (at, bt, av, bv) = tick_parity(&code, true, 0x0000, 0x0300_0000, &[], code.len());
    assert_eq!((at, bt), (at, at), "ticks diverge");
    assert_eq!((av, bv), (av, av), "timer span diverges");
}

#[test]
fn conditional_branch_loop_tick_parity() {
    let arm = [
        0xE3A0_0000,
        0xE280_0001,
        0xE350_0003,
        0x1AFF_FFFC,
        0xE3A0_1007,
    ];
    let thumb = [0x2000, 0x3001, 0x2803, 0xD1FC, 0x2107];

    let (at, bt, av, bv) = tick_parity(&arm, false, 0x4014, 0x0300_0000, &[], 11);
    assert_eq!((at, bt), (at, at), "ARM ticks diverge");
    assert_eq!((av, bv), (av, av), "ARM timer span diverges");

    let (at, bt, av, bv) = tick_parity(&thumb, true, 0x4014, 0x0300_0000, &[], 11);
    assert_eq!((at, bt), (at, at), "Thumb ticks diverge");
    assert_eq!((av, bv), (av, av), "Thumb timer span diverges");
}

#[test]
fn register_offset_load_store_tick_parity() {
    let (at, bt, av, bv) = tick_parity(
        &THUMB_REG_LS_CORPUS,
        true,
        0x4014,
        0x0300_0000,
        &[(0, 0x0000_80FF), (1, 0x0300_0100), (2, 4)],
        THUMB_REG_LS_CORPUS.len(),
    );
    assert_eq!((at, bt), (at, at), "ticks diverge");
    assert_eq!((av, bv), (av, av), "timer span diverges");
}
