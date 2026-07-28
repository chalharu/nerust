use crate::cpu_core::Lr35902Cpu;
use crate::cpu_core::Phase;
use crate::memory::GbcMemoryBus;

const BASE: u16 = 0xC000;

/// Create a CPU with a program at WRAM[BASE..], execute one full
/// instruction, and return the CPU state.
fn step_until_done(cpu: &mut Lr35902Cpu, bus: &mut GbcMemoryBus) {
    let start_pc = cpu.registers.pc;
    let start_t = cpu.t_cycle;
    for _ in 0..96 {
        let was_exec = !matches!(cpu.phase, Phase::FetchOpcode);
        cpu.step_t_cycle(bus);
        if cpu.t_cycle == start_t
            && matches!(cpu.phase, Phase::FetchOpcode)
            && (was_exec || cpu.registers.pc != start_pc)
        {
            return;
        }
    }
    panic!("instruction at {:04X} did not complete", cpu.registers.pc);
}

/// Load a ROM into WRAM at BASE and return (cpu, bus).
fn setup(rom: &[u8]) -> (Lr35902Cpu, GbcMemoryBus) {
    let mut bus = GbcMemoryBus::new([0; 0x100], false);
    for (i, &b) in rom.iter().enumerate() {
        bus.write(BASE + i as u16, b);
    }
    let mut cpu = Lr35902Cpu::new();
    cpu.registers.pc = BASE;
    (cpu, bus)
}

/// Run one instruction from a ROM.
fn exec_one(opcode: u8, operands: &[u8]) -> Lr35902Cpu {
    let (mut cpu, mut bus) = setup(&[]);
    bus.write(BASE, opcode);
    for (i, &b) in operands.iter().enumerate() {
        bus.write(BASE + 1 + i as u16, b);
    }
    cpu.registers.pc = BASE;
    step_until_done(&mut cpu, &mut bus);
    cpu
}

/// Run N instructions from a ROM program.
fn exec_n(rom: &[u8], n: usize) -> Lr35902Cpu {
    let (mut cpu, mut bus) = setup(rom);
    for _ in 0..n {
        step_until_done(&mut cpu, &mut bus);
    }
    cpu
}

// ── Timing verification (M-cycle counts vs spec) ─────────

fn count_tcycles(rom: &[u8]) -> usize {
    let (mut cpu, mut bus) = setup(rom);
    let start_t = cpu.t_cycle;
    let mut n = 0;
    loop {
        cpu.step_t_cycle(&mut bus);
        n += 1;
        if cpu.t_cycle == start_t && matches!(cpu.phase, Phase::FetchOpcode) { break; }
        if n > 48 { panic!("did not complete at PC={:04X}", cpu.registers.pc); }
    }
    eprintln!("count_tcycles({:02X?}) = {} T-cycles", rom, n);
    n
}

#[test]
fn timing_nop() { assert_eq!(count_tcycles(&[0x00]), 4, "NOP 1M"); }
#[test]
fn timing_xor_a() { assert_eq!(count_tcycles(&[0xAF]), 4, "XOR A 1M"); }
#[test]
fn timing_ld_hl_a() { assert_eq!(count_tcycles(&[0x77]), 8, "LD (HL),A 2M"); }
#[test]
fn timing_or_hl() { assert_eq!(count_tcycles(&[0xB6]), 8, "OR (HL) 2M"); }
#[test]
fn timing_push_af() { assert_eq!(count_tcycles(&[0xF5]), 16, "PUSH AF 4M"); }
#[test]
fn timing_pop_af() { assert_eq!(count_tcycles(&[0xF1]), 12, "POP AF 3M"); }
#[test]
fn timing_ld_a_d8() { assert_eq!(count_tcycles(&[0x3E, 0x42]), 8, "LD A,d8 2M"); }
#[test]
fn timing_ld_hl_d16() { assert_eq!(count_tcycles(&[0x21, 0x05, 0xFF]), 12, "LD HL,d16 3M"); }
#[test]
fn timing_call() { assert_eq!(count_tcycles(&[0xCD, 0x00, 0xC0]), 24, "CALL 6M"); }
#[test]
fn timing_ret() { assert_eq!(count_tcycles(&[0xC9]), 16, "RET 4M"); }
#[test]
fn timing_jr() { assert_eq!(count_tcycles(&[0x18, 0x00]), 12, "JR e 3M"); }
#[test]
fn timing_jr_nz_taken() { 
    let c = count_tcycles(&[0x18, 0x02, 0x00, 0x00]);
    assert_eq!(c, 12, "JR e taken 3M");
}
#[test]
fn timing_ret_nc_taken() {
    // RET NC with carry=0 (taken) = 5M. Ensure carry is 0.
    let (mut cpu, mut bus) = setup(&[0x37, 0x3F, 0xD0]); // SCF, CCF, RET NC
    // After SCF: C=1. After CCF: C=0. RET NC: taken.
    let start_t = cpu.t_cycle;
    let mut n = 0;
    for _ in 0..3 {
        loop {
            cpu.step_t_cycle(&mut bus);
            n += 1;
            if cpu.t_cycle == start_t && matches!(cpu.phase, Phase::FetchOpcode) { break; }
            if n > 48 { panic!("did not complete"); }
        }
    }
    // All 3 instructions: SCF(1M) + CCF(1M) + RET NC taken(5M) = 7M = 28T
    assert_eq!(n, 28, "SCF+CCF+RET NC taken = 7M=28T");
}

// ── NOP ──────────────────────────────────────────────────

#[test]
fn nop() {
    let cpu = exec_one(0x00, &[]);
    assert_eq!(cpu.registers.pc, BASE + 1);
}

// ── LD r8, d8 ────────────────────────────────────────────

#[test]
fn ld_a_d8() {
    let cpu = exec_one(0x3E, &[0x42]);
    assert_eq!(cpu.registers.a, 0x42);
}

// ── LD r16, d16 ──────────────────────────────────────────

#[test]
fn ld_bc_d16() {
    let cpu = exec_one(0x01, &[0x34, 0x12]);
    assert_eq!(cpu.registers.bc(), 0x1234);
}

// ── LD (HL), A / LD A, (HL) ─────────────────────────────

#[test]
fn ld_hl_a_and_readback() {
    let cpu = exec_n(&[0x3E, 0x55, 0x21, 0x00, 0xC0, 0x77, 0x7E], 4);
    assert_eq!(cpu.registers.a, 0x55);
}

// ── INC r8 ───────────────────────────────────────────────

#[test]
fn inc_a() {
    let cpu = exec_n(&[0x3E, 0x05, 0x3C], 2);
    assert_eq!(cpu.registers.a, 6);
    assert!(!cpu.registers.z_flag());
}

#[test]
fn inc_a_to_zero() {
    let cpu = exec_n(&[0x3E, 0xFF, 0x3C], 2);
    assert_eq!(cpu.registers.a, 0);
    assert!(cpu.registers.z_flag());
}

// ── DEC r8 ───────────────────────────────────────────────

#[test]
fn dec_a() {
    let cpu = exec_n(&[0x3E, 0x05, 0x3D], 2);
    assert_eq!(cpu.registers.a, 4);
    assert!(cpu.registers.n_flag());
}

// ── ADD A, B ─────────────────────────────────────────────

#[test]
fn add_a_b() {
    let cpu = exec_n(&[0x3E, 0x10, 0x06, 0x08, 0x80], 3);
    assert_eq!(cpu.registers.a, 0x18);
    assert!(!cpu.registers.c_flag());
}

#[test]
fn add_with_half_carry() {
    let cpu = exec_n(&[0x3E, 0x0F, 0x06, 0x01, 0x80], 3);
    assert_eq!(cpu.registers.a, 0x10);
    assert!(cpu.registers.h_flag());
}

#[test]
fn add_with_carry() {
    let cpu = exec_n(&[0x3E, 0xF0, 0x06, 0x20, 0x80], 3);
    assert_eq!(cpu.registers.a, 0x10);
    assert!(cpu.registers.c_flag());
}

// ── SUB A, B ─────────────────────────────────────────────

#[test]
fn sub_a_b() {
    let cpu = exec_n(&[0x3E, 0x20, 0x06, 0x05, 0x90], 3);
    assert_eq!(cpu.registers.a, 0x1B);
    assert!(cpu.registers.n_flag());
}

#[test]
fn sub_with_borrow() {
    let cpu = exec_n(&[0x3E, 0x05, 0x06, 0x10, 0x90], 3);
    assert_eq!(cpu.registers.a, 0xF5);
    assert!(cpu.registers.c_flag());
}

// ── AND / XOR / OR / CP ──────────────────────────────────

#[test]
fn and_a_b() {
    let cpu = exec_n(&[0x3E, 0xF0, 0x06, 0x0F, 0xA0], 3);
    assert_eq!(cpu.registers.a, 0x00);
    assert!(cpu.registers.z_flag());
    assert!(cpu.registers.h_flag());
}

#[test]
fn xor_a_b() {
    let cpu = exec_n(&[0x3E, 0xFF, 0x06, 0x0F, 0xA8], 3);
    assert_eq!(cpu.registers.a, 0xF0);
}

#[test]
fn or_a_b() {
    let cpu = exec_n(&[0x3E, 0xF0, 0x06, 0x0F, 0xB0], 3);
    assert_eq!(cpu.registers.a, 0xFF);
}

#[test]
fn cp_a_b_preserves_a() {
    let cpu = exec_n(&[0x3E, 0x42, 0x06, 0x42, 0xB8], 3);
    assert_eq!(cpu.registers.a, 0x42);
    assert!(cpu.registers.z_flag());
}

// ── ADC / SBC ────────────────────────────────────────────

#[test]
fn adc_with_carry() {
    let cpu = exec_n(&[0x3E, 0x10, 0x37, 0x06, 0x10, 0x88], 4);
    assert_eq!(cpu.registers.a, 0x21);
}

#[test]
fn sbc_with_carry() {
    let cpu = exec_n(&[0x3E, 0x20, 0x37, 0x06, 0x10, 0x98], 4);
    assert_eq!(cpu.registers.a, 0x0F);
}

// ── INC/DEC r16 ──────────────────────────────────────────

#[test]
fn inc_bc() {
    let cpu = exec_n(&[0x01, 0xFF, 0xFF, 0x03], 2);
    assert_eq!(cpu.registers.bc(), 0x0000);
}

#[test]
fn dec_bc() {
    let cpu = exec_n(&[0x01, 0x00, 0x00, 0x0B], 2);
    assert_eq!(cpu.registers.bc(), 0xFFFF);
}

// ── ADD HL, BC ───────────────────────────────────────────

#[test]
fn add_hl_bc() {
    let cpu = exec_n(&[0x21, 0x00, 0x10, 0x01, 0x00, 0x01, 0x09], 3);
    assert_eq!(cpu.registers.hl(), 0x1100);
    assert!(!cpu.registers.c_flag());
}

#[test]
fn add_hl_bc_overflow() {
    let cpu = exec_n(&[0x21, 0x00, 0xF0, 0x01, 0x00, 0x20, 0x09], 3);
    assert_eq!(cpu.registers.hl(), 0x1000);
    assert!(cpu.registers.c_flag());
}

// ── Step counts ───────────────────────────────────────────

#[test]
fn ldh_a_a8_takes_12_t_cycles() {
    let (mut cpu, mut bus) = setup(&[0xF0, 0x05]);
    let start_t = cpu.t_cycle;
    let mut n = 0;
    loop {
        cpu.step_t_cycle(&mut bus);
        n += 1;
        if cpu.t_cycle == start_t && matches!(cpu.phase, Phase::FetchOpcode) {
            break;
        }
        if n > 48 {
            panic!("did not complete");
        }
    }
    assert_eq!(n, 12, "LDH A,(a8) should take 12 T-cycles (3 M-cycles)");
}

#[test]
fn ld_a_a16_takes_16_t_cycles() {
    let (mut cpu, mut bus) = setup(&[0xFA, 0x00, 0xC0]);
    let start_t = cpu.t_cycle;
    let mut n = 0;
    loop {
        cpu.step_t_cycle(&mut bus);
        n += 1;
        if cpu.t_cycle == start_t && matches!(cpu.phase, Phase::FetchOpcode) {
            break;
        }
        if n > 48 {
            panic!("did not complete");
        }
    }
    assert_eq!(n, 16, "LD A,(a16) should take 16 T-cycles (4 M-cycles)");
}

// ── JR (unconditional) ──────────────────────────────────

#[test]
fn jr_forward() {
    let cpu = exec_one(0x18, &[0x02]);
    assert_eq!(cpu.registers.pc, BASE + 4);
}

// ── JP a16 ───────────────────────────────────────────────

#[test]
fn jp_a16() {
    let cpu = exec_one(0xC3, &[0x00, 0xC0]);
    assert_eq!(cpu.registers.pc, 0xC000);
}

// ── CALL ─────────────────────────────────────────────────

#[test]
fn call_pushes_return_address() {
    let cpu = exec_one(0xCD, &[0x10, 0xC0]);
    assert_eq!(cpu.registers.pc, 0xC010);
}

// ── RST ──────────────────────────────────────────────────

#[test]
fn rst_vectors() {
    let vectors = [0xC7, 0xCF, 0xD7, 0xDF, 0xE7, 0xEF, 0xF7, 0xFF];
    let expected = [0x00, 0x08, 0x10, 0x18, 0x20, 0x28, 0x30, 0x38];
    for (&op, &exp) in vectors.iter().zip(expected.iter()) {
        let cpu = exec_one(op, &[]);
        assert_eq!(cpu.registers.pc, exp, "RST {:02X}", op);
    }
}

// ── PUSH / POP ───────────────────────────────────────────

#[test]
fn push_pop_hl() {
    let cpu = exec_n(&[0x21, 0xEF, 0xBE, 0xE5, 0xE1], 3);
    assert_eq!(cpu.registers.hl(), 0xBEEF);
    assert_eq!(cpu.registers.sp, 0xFFFE);
}

// ── LD HL, SP+e ──────────────────────────────────────────

#[test]
fn ld_hl_sp_e() {
    let cpu = exec_n(&[0x31, 0x00, 0xC1, 0xF8, 0x10], 2);
    assert_eq!(cpu.registers.hl(), 0xC110);
}

// ── CB prefix ────────────────────────────────────────────

#[test]
fn cb_bit_test() {
    let cpu = exec_n(&[0x3E, 0x80, 0xCB, 0x7F], 2);
    assert!(!cpu.registers.z_flag());
    assert!(cpu.registers.h_flag());
}

#[test]
fn cb_res_clears_bit() {
    let cpu = exec_n(&[0x3E, 0x80, 0xCB, 0xBF], 2);
    assert_eq!(cpu.registers.a, 0x00);
}

#[test]
fn cb_set_sets_bit() {
    let cpu = exec_n(&[0x3E, 0x00, 0xCB, 0xC7], 2);
    assert_eq!(cpu.registers.a, 0x01);
}

#[test]
fn cb_rlc_through_carry() {
    let cpu = exec_n(&[0x3E, 0x80, 0xCB, 0x07], 2);
    assert_eq!(cpu.registers.a, 0x01);
    assert!(cpu.registers.c_flag());
}

// ── Misc ──────────────────────────────────────────────────

#[test]
fn daa_adjusts_after_addition() {
    let cpu = exec_n(&[0x3E, 0x09, 0xC6, 0x08, 0x27], 3);
    assert_eq!(cpu.registers.a, 0x17);
}

#[test]
fn scf_sets_carry() {
    let cpu = exec_one(0x37, &[]);
    assert!(cpu.registers.c_flag());
}

#[test]
fn cpl_flips_a() {
    let cpu = exec_n(&[0x3E, 0x55, 0x2F], 2);
    assert_eq!(cpu.registers.a, 0xAA);
}

#[test]
fn ld_hli_a_and_readback() {
    let cpu = exec_n(&[0x21, 0x00, 0xC0, 0x3E, 0x77, 0x22, 0x2A], 4);
    assert_eq!(cpu.registers.hl(), 0xC002); // HL advanced twice
}

#[test]
fn ld_hld_a() {
    let cpu = exec_n(&[0x21, 0x02, 0xC0, 0x3E, 0x88, 0x32], 3);
    assert_eq!(cpu.registers.hl(), 0xC001);
}
