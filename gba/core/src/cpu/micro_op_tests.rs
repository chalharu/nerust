//! Expansion shape/gate/manifest tests for the unified micro-op engine.
//! Lives in a sibling of `micro_op` (not a child) so the file dependency
//! stays one-way: `cpu -> micro_op`, `cpu -> micro_op_tests`,
//! `micro_op_tests -> {micro_op, cpu}`, with nothing pointing back at
//! `micro_op_tests`.
use super::micro_op::{
    expand_arm, expand_arm_dp_reg, expand_arm_mul, expand_arm_single, expand_thumb,
    expand_thumb_alu_rest,
};
use crate::cpu_registers::CpuRegisters;

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
    // Register shift: commit + 1I. ADD PC: commit + 2I.
    let ops = expand_thumb(0x41C1, &regs).expect("ror expands");
    assert_eq!(ops.len(), 2);
    let ops = expand_thumb(0x4487, &regs).expect("add-pc expands");
    assert_eq!(ops.len(), 3);
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

/// ARM halfword/single-transfer expansion shapes and gates.
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
    // ARM BX: refill pair + commit.
    let ops = expand_arm(0xE12F_FF13, &regs).expect("arm bx expands");
    assert_eq!(ops.len(), 3);
    // DP-imm with Rn==PC reads the execute-stage PC (bus-free).
    let ops = expand_arm(0xE28F_300C, &regs).expect("add-pc expands");
    assert_eq!(ops.len(), 1);
}

/// Coverage manifest (see body).
#[test]
fn coverage_manifest() {
    let regs = CpuRegisters::post_bios();
    // Formerly legacy ARM: SWI, coprocessor UND.
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
