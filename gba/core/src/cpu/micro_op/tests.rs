//! Micro-op engine tests: execute-effect pins (absolute values) plus
//! expansion shape/gate/manifest tests. All live here, next to the
//! engine, importing the pieces explicitly.
use super::MicroOp;
use super::apply::{apply_mul, apply_op};
use super::apply_arm::{apply_dp_reg, apply_psr, apply_swp, apply_trap_swi, apply_trap_und};
use super::expand_arm::{expand_arm, expand_arm_dp_reg, expand_arm_mul, expand_arm_single};
use super::expand_thumb::{expand_thumb, expand_thumb_alu_rest};
use crate::cpu::semantics::condition_passed;
use crate::cpu_registers::CpuRegisters;
use crate::memory::GbaMemoryBus;
use std::collections::VecDeque;

/// Drive expanded ops without retire (no flush/pc-advance): pins
/// execute effects at absolute values.
fn run_ops(
    regs: &mut CpuRegisters,
    bus: &mut GbaMemoryBus,
    ops: super::MicroOpVec,
    pc: u32,
    is_thumb: bool,
) {
    let mut queue: VecDeque<MicroOp> = ops.into_iter().collect();
    while let Some(op) = queue.pop_front() {
        apply_op(regs, bus, op, pc, is_thumb);
    }
}

fn run_arm(regs: &mut CpuRegisters, bus: &mut GbaMemoryBus, instr: u32) {
    let pc = regs.pc();
    let ops = expand_arm(instr, regs).expect("arm expands");
    run_ops(regs, bus, ops, pc, false);
}

fn run_thumb(regs: &mut CpuRegisters, bus: &mut GbaMemoryBus, instr: u16) {
    let pc = regs.pc();
    let ops = expand_thumb(instr, regs).expect("thumb expands");
    run_ops(regs, bus, ops, pc, true);
}

#[test]
fn trap_swi_enters_svc_with_banked_lr() {
    let mut regs = CpuRegisters::post_bios();
    regs.set_pc(0x08000008);
    let old_cpsr = regs.cpsr();
    let mut bus = GbaMemoryBus::new();
    // SWI 0xFF is unhandled -> SVC vector.
    assert_eq!(apply_trap_swi(&mut regs, &mut bus, 0xFF, false), 3);
    assert_eq!(regs.cpsr_mode(), 0x13);
    assert_eq!(regs.spsr(), old_cpsr);
    assert_eq!(regs.lr(), 0x08000004);
    assert_eq!(regs.pc(), 0x08);
}

#[test]
fn trap_swi_uses_hle_number() {
    let mut regs = CpuRegisters::post_bios();
    regs.set_pc(0x08000008);
    let mut bus = GbaMemoryBus::new();
    let expected = bus.bios_checksum();
    apply_trap_swi(&mut regs, &mut bus, 0x0D, false);
    assert_eq!(regs.r(0), expected);
    assert_eq!(regs.cpsr_mode(), 0x1F);
    assert_eq!(regs.pc(), 0x08000004);
}

#[test]
fn trap_swi_thumb_returns_after_swi() {
    let mut regs = CpuRegisters::post_bios();
    regs.set_cpsr(regs.cpsr() | (1 << 5));
    regs.set_pc(0x08000006);
    let mut bus = GbaMemoryBus::new();
    apply_trap_swi(&mut regs, &mut bus, 0x0D, true);
    assert_eq!(regs.pc(), 0x08000004);
    assert!(regs.take_pc_written());
}

#[test]
fn trap_und_enters_undefined_exception() {
    let mut regs = CpuRegisters::post_bios();
    regs.set_pc(0x08000008);
    let old_cpsr = regs.cpsr();
    let mut bus = GbaMemoryBus::new();
    assert_eq!(apply_trap_und(&mut regs, false), 4);
    assert_eq!(regs.cpsr_mode(), 0x1B);
    assert_eq!(regs.spsr(), old_cpsr);
    assert_eq!(regs.lr(), 0x08000004);
    assert_eq!(regs.pc(), 0x04);
    // Thumb form uses the -2 return address.
    let mut regs = CpuRegisters::post_bios();
    regs.set_cpsr(regs.cpsr() | (1 << 5));
    regs.set_pc(0x08000006);
    assert_eq!(apply_trap_und(&mut regs, true), 4);
    assert_eq!(regs.cpsr_mode(), 0x1B);
    assert_eq!(regs.lr(), 0x08000004);
    assert_eq!(regs.pc(), 0x04);
    let _ = &mut bus;
}

#[test]
fn block_empty_thumb_transfers_pc_and_writes_back_0x40() {
    let mut regs = CpuRegisters::post_bios();
    regs.set_cpsr(regs.cpsr() | (1 << 5));
    regs.set_pc(0x08000104);
    regs.set_r(0, 0x03000000);
    let mut bus = GbaMemoryBus::new();
    bus.write32(0x03000000, 0x08000201);
    // Empty LDMIA R0: loads PC, Rb += 0x40 (LSB ignored on
    // ARMv4T like POP {PC}).
    run_thumb(&mut regs, &mut bus, 0xC800);
    assert_eq!(regs.pc(), 0x08000200);
    assert_eq!(regs.r(0), 0x03000040);
}

#[test]
fn block_empty_arm_stores_pc_and_loads_back() {
    let mut regs = CpuRegisters::post_bios();
    regs.set_pc(0x08000000);
    regs.set_r(0, 0x03000200);
    regs.set_r(1, 0x03000200);
    let mut bus = GbaMemoryBus::new();
    // Empty STMIA R0!: stores PC+4, R0 += 0x40.
    run_arm(&mut regs, &mut bus, 0xE8A0_0000);
    assert_eq!(bus.read32(0x03000200), 0x08000004);
    assert_eq!(regs.r(0), 0x03000240);
    // Empty LDMIA R1!: loads PC, R1 += 0x40.
    bus.write32(0x03000200, 0x03000100);
    run_arm(&mut regs, &mut bus, 0xE8B1_0000);
    assert_eq!(regs.pc(), 0x03000100);
    assert_eq!(regs.r(1), 0x03000240);
}

#[test]
fn psr_updates_flags_field() {
    let mut regs = CpuRegisters::post_bios();
    regs.set_r(0, 0xFF000000);
    apply_psr(&mut regs, 0xE128F000); // MSR CPSR_f, R0
    assert_eq!(regs.cpsr() & 0xF0000000, 0xF0000000);
    assert_eq!(regs.cpsr() & 0x0F000000, 0);
    assert_eq!(regs.cpsr_mode(), 0x1F);
}

#[test]
fn psr_user_msr_cannot_change_control_field() {
    let mut regs = CpuRegisters::post_bios();
    regs.set_cpsr(0x10);
    regs.set_r(0, 0x13);
    apply_psr(&mut regs, 0xE121F000); // MSR CPSR_c, R0
    assert_eq!(regs.cpsr_mode(), 0x10);
}

#[test]
fn swp_exchanges_word_and_byte() {
    let mut regs = CpuRegisters::post_bios();
    let mut bus = GbaMemoryBus::new();
    bus.write32(0x03000000, 0x11223344);
    regs.set_r(0, 0x03000000);
    regs.set_r(1, 0xAABBCCDD);
    apply_swp(&mut regs, &mut bus, 0xE1002091); // SWP R2, R1, [R0]
    assert_eq!(regs.r(2), 0x11223344);
    assert_eq!(bus.read32(0x03000000), 0xAABBCCDD);
    bus.write8(0x03000000, 0x44);
    regs.set_r(1, 0xDD);
    apply_swp(&mut regs, &mut bus, 0xE1402091); // SWPB R2, R1, [R0]
    assert_eq!(regs.r(2), 0x44);
    assert_eq!(bus.read8(0x03000000), 0xDD);
}

#[test]
fn mul_simple() {
    let mut regs = CpuRegisters::post_bios();
    let mut bus = GbaMemoryBus::new();
    regs.set_r(1, 3);
    regs.set_r(2, 4);
    regs.set_r(0, 0);
    apply_mul(&mut regs, &mut bus, 0xE0000291); // MUL R0, R2, R1
    assert_eq!(regs.r(0), 12);
}

#[test]
fn dp_add_with_carry_wraps() {
    let mut regs = CpuRegisters::post_bios();
    regs.set_r(0, 0xFFFFFFFF);
    // ADDS R0, R0, #1 wraps to 0 with C=1.
    apply_dp_reg(&mut regs, 0xE2900001);
    assert_eq!(regs.r(0), 0);
    assert!(regs.cpsr_c());
}

#[test]
fn dp_mov_immediate() {
    let mut regs = CpuRegisters::post_bios();
    apply_dp_reg(&mut regs, 0xE3A000FF); // MOV R0, #0xFF
    assert_eq!(regs.r(0), 0xFF);
}

#[test]
fn dp_tst_preserves_overflow() {
    let mut regs = CpuRegisters::post_bios();
    regs.set_cpsr_v(true);
    regs.set_r(0, 1);
    apply_dp_reg(&mut regs, 0xE3100001); // TST R0,#1
    assert!(regs.cpsr_v());
}

#[test]
fn dp_add_sets_and_clears_overflow() {
    let mut regs = CpuRegisters::post_bios();
    regs.set_r(0, 0x7FFFFFFF);
    apply_dp_reg(&mut regs, 0xE2900001); // ADDS R0,R0,#1
    assert!(regs.cpsr_v());
    regs.set_r(0, 0);
    apply_dp_reg(&mut regs, 0xE2900001);
    assert!(!regs.cpsr_v());
}

#[test]
fn dp_rotated_immediates_build_full_word() {
    let mut regs = CpuRegisters::post_bios();
    for instruction in [0xE3A000FF, 0xE3800CFF, 0xE38008FF, 0xE380047F] {
        apply_dp_reg(&mut regs, instruction);
    }
    assert_eq!(regs.r(0), 0x7FFFFFFF);
}

#[test]
fn dp_register_shift_reads_pc_plus_12() {
    let mut regs = CpuRegisters::post_bios();
    regs.set_pc(0x08000108);
    regs.set_r(0, 0);
    apply_dp_reg(&mut regs, 0xE1A0001F); // MOV R0,PC,LSL R0
    assert_eq!(regs.r(0), 0x0800010C);
}

#[test]
fn branch_forward() {
    let mut regs = CpuRegisters::post_bios();
    let mut bus = GbaMemoryBus::new();
    regs.set_pc(0x08000000);
    // Architectural PC is current instruction + 8.
    run_arm(&mut regs, &mut bus, 0xEA000002);
    assert_eq!(regs.pc(), 0x08000008);
}

#[test]
fn thumb_branch_ranges() {
    let mut regs = CpuRegisters::post_bios();
    regs.set_cpsr(regs.cpsr() | (1 << 5));
    let mut bus = GbaMemoryBus::new();
    regs.set_pc(0x08003F08);
    run_thumb(&mut regs, &mut bus, 0xE317);
    assert_eq!(regs.pc(), 0x08004536);
    regs.set_pc(0x08001000);
    run_thumb(&mut regs, &mut bus, 0xE7FF);
    assert_eq!(regs.pc(), 0x08000FFE);
}

#[test]
fn load_store_immediate_roundtrip() {
    let mut regs = CpuRegisters::post_bios();
    let mut bus = GbaMemoryBus::new();
    regs.set_r(1, 0x02000000);
    regs.set_r(0, 0x12345678);
    run_arm(&mut regs, &mut bus, 0xE5810004); // STR R0, [R1, #4]
    run_arm(&mut regs, &mut bus, 0xE5912004); // LDR R2, [R1, #4]
    assert_eq!(regs.r(2), 0x12345678);
}

#[test]
fn halfword_load_store_roundtrip() {
    let mut regs = CpuRegisters::post_bios();
    let mut bus = GbaMemoryBus::new();
    regs.set_r(1, 0x02000000);
    regs.set_r(0, 0x1234);
    run_arm(&mut regs, &mut bus, 0xE1C100B0); // STRH R0, [R1]
    regs.set_r(0, 0);
    run_arm(&mut regs, &mut bus, 0xE1D100B0); // LDRH R0, [R1]
    assert_eq!(regs.r(0) & 0xFFFF, 0x1234);
}

#[test]
fn byte_imm_load_store_roundtrip() {
    let mut regs = CpuRegisters::post_bios();
    let mut bus = GbaMemoryBus::new();
    regs.set_r(1, 0x02000000);
    regs.set_r(0, 0xAB);
    run_thumb(&mut regs, &mut bus, 0x7008); // STRB R0, [R1]
    regs.set_r(0, 0);
    run_thumb(&mut regs, &mut bus, 0x7808); // LDRB R0, [R1]
    assert_eq!(regs.r(0) & 0xFF, 0xAB);
}

#[test]
fn signed_loads_extend_sign() {
    let mut regs = CpuRegisters::post_bios();
    let mut bus = GbaMemoryBus::new();
    regs.set_r(1, 0x03000000);
    regs.set_r(2, 0);
    bus.write16(0x03000000, 0x80FF);
    run_thumb(&mut regs, &mut bus, 0x5688); // LDRSB R0,[R1,R2]
    assert_eq!(regs.r(0), 0xFFFFFFFF);
    run_thumb(&mut regs, &mut bus, 0x5E88); // LDRSH R0,[R1,R2]
    assert_eq!(regs.r(0), 0xFFFF80FF);
}

#[test]
fn block_roundtrip() {
    let mut regs = CpuRegisters::post_bios();
    let mut bus = GbaMemoryBus::new();
    regs.set_r(0, 0x02000000);
    regs.set_r(1, 0x11111111);
    regs.set_r(2, 0x22222222);
    run_arm(&mut regs, &mut bus, 0xE8A00006); // STMIA R0!, {R1,R2}
    regs.set_r(0, 0x02000000);
    regs.set_r(3, 0);
    regs.set_r(4, 0);
    run_arm(&mut regs, &mut bus, 0xE8B00018); // LDMIA R0, {R3,R4}
    assert_eq!(regs.r(3), 0x11111111);
    assert_eq!(regs.r(4), 0x22222222);
}

#[test]
fn block_unaligned_base() {
    let mut regs = CpuRegisters::post_bios();
    let mut bus = GbaMemoryBus::new();
    let base = 0x02000100;
    regs.set_r(0, 32);
    regs.set_r(1, 64);
    regs.set_r(2, base + 3);
    regs.set_r(3, base - 5);
    run_arm(&mut regs, &mut bus, 0xE9220003); // STMDB R2!,{R0,R1}
    run_arm(&mut regs, &mut bus, 0xE8930030); // LDMIA R3,{R4,R5}
    assert_eq!(regs.r(4), 32);
    assert_eq!(regs.r(5), 64);
    assert_eq!(regs.r(2), regs.r(3));
}

#[test]
fn alu_format_reaches_native() {
    let mut regs = CpuRegisters::post_bios();
    regs.set_r(0, 1);
    regs.set_r(1, 2);
    let mut bus = GbaMemoryBus::new();
    run_thumb(&mut regs, &mut bus, 0x4308); // ORR R0,R1
    assert_eq!(regs.r(0), 3);
}

#[test]
fn pop_and_ldmia_reachable() {
    let mut regs = CpuRegisters::post_bios();
    let mut bus = GbaMemoryBus::new();
    regs.set_sp(0x03000000);
    bus.write32(0x03000000, 0x12345678);
    run_thumb(&mut regs, &mut bus, 0xBC01); // POP {R0}
    assert_eq!(regs.r(0), 0x12345678);
    regs.set_r(1, 0x03000004);
    bus.write32(0x03000004, 0xCAFEBABE);
    run_thumb(&mut regs, &mut bus, 0xC904); // LDMIA R1!, {R2}
    assert_eq!(regs.r(2), 0xCAFEBABE);
}

#[test]
fn multiply_long_reaches_native() {
    let mut regs = CpuRegisters::post_bios();
    let mut bus = GbaMemoryBus::new();
    regs.set_r(0, 3);
    regs.set_r(1, 4);
    run_arm(&mut regs, &mut bus, 0xE0832190); // UMULL R2,R3,R0,R1
    assert_eq!(regs.r(2), 12);
    assert_eq!(regs.r(3), 0);
}

#[test]
fn cond_codes() {
    assert!(condition_passed(1 << 30, 0x0));
    assert!(!condition_passed(0, 0x0));
    assert!(condition_passed(0, 0xE));
}

// Expansion shape/gate/manifest tests (ported from the former
// sibling `micro_op_tests` module so all micro-op tests live here).

#[test]
fn empty_push_pop_expands() {
    fn regs_with_sp(sp: u32) -> CpuRegisters {
        let mut regs = CpuRegisters::post_bios();
        regs.set_sp(sp);
        regs
    }
    let regs = regs_with_sp(0x0300_7F00);
    // Empty PUSH: Start + Empty + End + 1 trailing = 4.
    let ops = expand_thumb(0xB400, &regs).expect("empty pushes expand");
    assert_eq!(ops.len(), 4);
    // Empty POP: Start + Empty + End + 5 trailing = 8.
    let ops = expand_thumb(0xBC00, &regs).expect("empty pops expand");
    assert_eq!(ops.len(), 8);
    let ops = expand_thumb(0xB40F, &regs).expect("non-empty pushes expand");
    // BlockStart + 4 words + BlockEnd + 1 trailing = 7.
    assert_eq!(ops.len(), 7);
}

/// Empty Thumb LDMIA/STMIA expansion shapes.
#[test]
fn empty_ldm_stm_expands() {
    let mut regs = CpuRegisters::post_bios();
    regs.set_r(1, 0x0300_0100);
    // Empty LDMIA/STMIA: single Empty op + trailing (4 for LDM, 1
    // for STM). Thumb LDMIA/STMIA use the 0xC000/0xC800 bases.
    let ops = expand_thumb(0xC000, &regs).expect("empty stmia expands");
    assert_eq!(ops.len(), 2);
    let ops = expand_thumb(0xC800, &regs).expect("empty ldmia expands");
    assert_eq!(ops.len(), 5);
    let ops = expand_thumb(0xC10F, &regs).expect("non-empty stmia expands");
    // BlockStart + 4 words + BlockEnd + 1 trailing = 7.
    assert_eq!(ops.len(), 7);
}

/// ARM block expansion gates (S-bit forms, empty list).
#[test]
fn arm_block_gates() {
    let mut regs = CpuRegisters::post_bios();
    regs.set_r(0, 0x0200_0000);
    // S bit now expands (user-bank forms, asserted below), as does
    // the empty list (asserted in the empty-forms test).
    let ops = expand_arm(0xE8B5_0018 | (1 << 22), &regs).expect("ldmia^ expands");
    // BlockStart + 2 words + BlockEnd + 2 trailing = 6.
    assert_eq!(ops.len(), 6);
    // Empty list now expands (plain and S-bit): single Empty op +
    // trailing (LDM 4, STM 1).
    let ops = expand_arm(0xE8A0_0000, &regs).expect("empty stmia expands");
    assert_eq!(ops.len(), 2);
    let ops = expand_arm(0xE8A0_0000 | (1 << 22), &regs).expect("empty stmia^ expands");
    assert_eq!(ops.len(), 2);
    let ops = expand_arm(0xE8A0_0006, &regs).expect("plain stmia expands");
    // BlockStart + 2 words + BlockEnd + 1 trailing = 5.
    assert_eq!(ops.len(), 5);
}

/// ARM DP-register branch gates (multiply/SWP/PSR/BX/halfword
/// keep their own branches).
#[test]
fn arm_dpreg_gates() {
    let regs = CpuRegisters::post_bios();
    // The DP-register branch itself must not misclaim multiply,
    // SWP, MRS/MSR, BX or halfword shapes (they have their own
    // branches); assert on the branch directly.
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
fn arm_mul_padding_matches_pinned_base() {
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
fn thumb_bl_bx_shapes_and_gates() {
    let mut regs = CpuRegisters::post_bios();
    regs.set_cpsr(regs.cpsr() | (1 << 5));
    regs.set_lr(0x0300_0005);
    // BL-high: single LR op. BL-low / BX: folded single commits
    // (2-cycle refill in their cost).
    let bl_hi = expand_thumb(0xF000, &regs).expect("bl-hi expands");
    assert_eq!(bl_hi.len(), 1);
    let bl_lo = expand_thumb(0xF806, &regs).expect("bl-lo expands");
    assert_eq!(bl_lo.len(), 1);
    let bx = expand_thumb(0x4770, &regs).expect("bx expands");
    assert_eq!(bx.len(), 1);
    // Hi-reg ADD beside BX is covered by the ALU-remainder branch
    // (asserted in thumb_alu_rest_shapes_and_gates).
}

/// Thumb ALU-remainder expansion shapes and gates.
#[test]
fn thumb_alu_rest_shapes_and_gates() {
    let mut regs = CpuRegisters::post_bios();
    regs.set_cpsr(regs.cpsr() | (1 << 5));
    // Single-cycle forms: commit only.
    for instr in [0x0041, 0x1881, 0x4001, 0x4580] {
        let ops = expand_thumb(instr, &regs).expect("alu form expands");
        assert_eq!(ops.len(), 1, "{instr:#06X}");
    }
    // Register shift / ADD PC: folded single commits (padding in cost).
    let ops = expand_thumb(0x41C1, &regs).expect("ror expands");
    assert_eq!(ops.len(), 1);
    let ops = expand_thumb(0x4487, &regs).expect("add-pc expands");
    assert_eq!(ops.len(), 1);
    // MUL and BX keep their own branches; SWI now traps.
    assert!(expand_thumb_alu_rest(0x4341).is_none());
    assert!(expand_thumb_alu_rest(0x4708).is_none());
    let ops = expand_thumb(0xDF00, &regs).expect("swi traps");
    assert_eq!(ops.len(), 1);
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
fn arm_singlerest_shapes() {
    let mut regs = CpuRegisters::post_bios();
    regs.set_r(1, 0x0200_0000);
    regs.set_r(2, 4);
    // Register offset: single folded access op (base 3 in its cost).
    let ops = expand_arm(0xE791_0002, &regs).expect("reg-offset expands");
    assert_eq!(ops.len(), 1);
    // R15 load: folded (base 5 in its cost). R15 store: folded.
    let ops = expand_arm(0xE59F_F000, &regs).expect("ldr-pc expands");
    assert_eq!(ops.len(), 1);
    let ops = expand_arm(0xE581_F004, &regs).expect("str-r15 expands");
    assert_eq!(ops.len(), 1);
    let ops = expand_arm(0xE1DF_00B0, &regs).expect("ldrh-pc expands");
    assert_eq!(ops.len(), 1);
}

/// ARM halfword/single-transfer expansion shapes and gates.
#[test]
fn arm_hwrest_shapes_and_gates() {
    let mut regs = CpuRegisters::post_bios();
    regs.set_r(1, 0x0200_0000);
    regs.set_r(2, 4);
    // Signed/reg-offset forms expand like the unsigned-imm ones
    // (single folded access op; base cycles in its cost).
    for instr in [
        0xE1D1_00F0,
        0xE1D1_00D0,
        0xE191_00B2,
        0xE191_00F2,
        0xE1DF_00F0,
    ] {
        let ops = expand_arm(instr, &regs).expect("halfword form expands");
        assert_eq!(ops.len(), 1, "{instr:#010X}");
    }
    let ops = expand_arm(0xE181_00B2, &regs).expect("strh-reg expands");
    assert_eq!(ops.len(), 1);
    // Multiply/SWP keep the decoder-first routing.
    assert!(expand_arm_single(0xE000_0090, &regs).is_none());
    assert!(expand_arm_single(0xE102_0091, &regs).is_none());
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
fn arm_psrbx_shapes_and_gates() {
    let regs = CpuRegisters::post_bios();
    // PSR forms: single commit op.
    for instr in [0xE10F_1000, 0xE129_F000, 0xE32B_F000] {
        let ops = expand_arm(instr, &regs).expect("psr expands");
        assert_eq!(ops.len(), 1, "{instr:#010X}");
    }
    // ARM BX: folded single commit (2-cycle refill in its cost).
    let ops = expand_arm(0xE12F_FF13, &regs).expect("arm bx expands");
    assert_eq!(ops.len(), 1);
    // DP-imm with Rn==PC reads the execute-stage PC (bus-free).
    let ops = expand_arm(0xE28F_300C, &regs).expect("add-pc expands");
    assert_eq!(ops.len(), 1);
}

/// Coverage manifest (see body).
#[test]
fn coverage_manifest() {
    let regs = CpuRegisters::post_bios();
    // Trap classes: SWI, coprocessor UND.
    for instr in [
        0xEF00_0000, // swi
        0xE7FF_FFFF, // swi (cond E, bit24 set)
        0xEE00_0010, // coprocessor (UND)
        0xEC00_0000, // coprocessor (UND)
    ] {
        assert!(expand_arm(instr, &regs).is_some(), "{instr:#010X}");
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
        0xE8A0_0000, // empty stmia
        0xE8B0_0000, // empty ldmia
        0xE8B5_0018, // ldmia
        0xE8F5_4018, // ldmia^ (S bit)
        0xE890_8000, // ldmia pc
        0xEA00_0001, // b
        0xEB00_0001, // bl (link)
        0xF3A0_0001, // nv (never executes)
    ] {
        assert!(expand_arm(instr, &aregs).is_some(), "{instr:#010X}");
    }
    // Covered Thumb representatives (one per class/form).
    let mut tregs = CpuRegisters::post_bios();
    tregs.set_cpsr(tregs.cpsr() | (1 << 5));
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
        0x7808, // ldrb imm-offset
        0x7008, // strb imm-offset
        0x8090, // strh
        0x9010, // str sp-relative
        0x9D10, // ldr sp-relative
        0xA004, // add pc
        0xB00A, // add sp
        0xB40F, // push
        0xB400, // empty push
        0xB100, // decoder gap (UND)
        0xB600, // decoder gap (UND)
        0xBE00, // decoder gap (UND)
        0xBC00, // empty pop
        0xC000, // empty stmia
        0xC800, // empty ldmia
        0xBCF0, // pop
        0xBD02, // pop pc
        0xC10F, // stmia
        0xCB18, // ldmia (base in list)
        0xD001, // cond branch
        0xDE00, // undefined trap
        0xDF00, // swi trap
        0xE001, // b
        0xF000, // bl high
        0xF806, // bl low
    ] {
        assert!(expand_thumb(instr, &tregs).is_some(), "{instr:#06X}");
    }
}
