use super::super::apply::apply_mul;
use super::super::apply_arm::{apply_dp_reg, apply_psr, apply_swp, apply_trap_swi, apply_trap_und};
use super::{run_arm, run_thumb, test_bus};
use crate::cpu_registers::CpuRegisters;

#[test]
fn trap_swi_enters_svc_with_banked_lr() {
    let mut regs = CpuRegisters::post_bios();
    regs.set_pc(0x08000008);
    let old_cpsr = regs.cpsr();
    let mut bus = test_bus();
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
    let mut bus = test_bus();
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
    let mut bus = test_bus();
    apply_trap_swi(&mut regs, &mut bus, 0x0D, true);
    assert_eq!(regs.pc(), 0x08000004);
    assert!(regs.take_pc_written());
}

#[test]
fn trap_und_enters_undefined_exception() {
    let mut regs = CpuRegisters::post_bios();
    regs.set_pc(0x08000008);
    let old_cpsr = regs.cpsr();
    let mut bus = test_bus();
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
    let mut bus = test_bus();
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
    let mut bus = test_bus();
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
    let mut bus = test_bus();
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
    let mut bus = test_bus();
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
    let mut bus = test_bus();
    regs.set_pc(0x08000000);
    // Architectural PC is current instruction + 8.
    run_arm(&mut regs, &mut bus, 0xEA000002);
    assert_eq!(regs.pc(), 0x08000008);
}

#[test]
fn thumb_branch_ranges() {
    let mut regs = CpuRegisters::post_bios();
    regs.set_cpsr(regs.cpsr() | (1 << 5));
    let mut bus = test_bus();
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
    let mut bus = test_bus();
    regs.set_r(1, 0x02000000);
    regs.set_r(0, 0x12345678);
    run_arm(&mut regs, &mut bus, 0xE5810004); // STR R0, [R1, #4]
    run_arm(&mut regs, &mut bus, 0xE5912004); // LDR R2, [R1, #4]
    assert_eq!(regs.r(2), 0x12345678);
}

#[test]
fn halfword_load_store_roundtrip() {
    let mut regs = CpuRegisters::post_bios();
    let mut bus = test_bus();
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
    let mut bus = test_bus();
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
    let mut bus = test_bus();
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
    let mut bus = test_bus();
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
    let mut bus = test_bus();
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
