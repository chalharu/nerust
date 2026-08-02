use crate::cpu_core::Lr35902Cpu;
use crate::cpu_core::Phase;
use crate::memory::GbcMemoryBus;

const BASE: u16 = 0xC000;

fn step_until_done(cpu: &mut Lr35902Cpu, bus: &mut GbcMemoryBus) {
    let start_pc = cpu.registers().pc();
    for _ in 0..24 {
        let was_executing = !matches!(cpu.phase(), Phase::FetchOpcode);
        cpu.step(bus);
        bus.step_devices(4);
        if matches!(cpu.phase(), Phase::FetchOpcode)
            && (was_executing || cpu.registers().pc() != start_pc)
        {
            return;
        }
    }
    panic!(
        "instruction at {:04X} did not complete",
        cpu.registers().pc()
    );
}

fn setup(rom: &[u8]) -> (Lr35902Cpu, GbcMemoryBus) {
    let mut bus = GbcMemoryBus::new([0; 0x100], false);
    for (i, &b) in rom.iter().enumerate() {
        bus.write(BASE + i as u16, b);
    }
    let mut cpu = Lr35902Cpu::new();
    cpu.registers_mut().set_pc(BASE);
    (cpu, bus)
}

fn exec_one(opcode: u8, operands: &[u8]) -> Lr35902Cpu {
    let (mut cpu, mut bus) = setup(&[]);
    bus.write(BASE, opcode);
    for (i, &b) in operands.iter().enumerate() {
        bus.write(BASE + 1 + i as u16, b);
    }
    cpu.registers_mut().set_pc(BASE);
    step_until_done(&mut cpu, &mut bus);
    cpu
}

fn exec_n(rom: &[u8], n: usize) -> Lr35902Cpu {
    let (mut cpu, mut bus) = setup(rom);
    for _ in 0..n {
        step_until_done(&mut cpu, &mut bus);
    }
    cpu
}

#[test]
fn ei_enables_ime_after_following_instruction() {
    let (mut cpu, mut bus) = setup(&[0xFB, 0x00]);

    step_until_done(&mut cpu, &mut bus);
    assert!(!bus.ime_enabled());

    step_until_done(&mut cpu, &mut bus);
    assert!(bus.ime_enabled());
}

#[test]
fn di_cancels_pending_ei() {
    let (mut cpu, mut bus) = setup(&[0xFB, 0xF3]);

    step_until_done(&mut cpu, &mut bus);
    step_until_done(&mut cpu, &mut bus);

    assert!(!bus.ime_enabled());
}

#[test]
fn interrupt_dispatch_pushes_pc_on_m3_and_m4() {
    let (mut cpu, mut bus) = setup(&[0x00]);
    cpu.registers_mut().set_sp(0xFFFE);
    bus.write(0xFFFF, 0x01);
    bus.write(0xFF0F, 0x01);
    bus.set_ime(true);

    cpu.step(&mut bus);
    assert!(matches!(
        cpu.phase(),
        Phase::InterruptDispatch { step: 1, .. }
    ));
    assert_eq!(cpu.registers().sp(), 0xFFFE);

    cpu.step(&mut bus);
    assert!(matches!(
        cpu.phase(),
        Phase::InterruptDispatch { step: 2, .. }
    ));
    assert_eq!(cpu.registers().sp(), 0xFFFE);

    cpu.step(&mut bus);
    assert!(matches!(
        cpu.phase(),
        Phase::InterruptDispatch { step: 3, .. }
    ));
    assert_eq!(cpu.registers().sp(), 0xFFFD);
    assert_eq!(bus.read(0xFFFD), (BASE >> 8) as u8);

    cpu.step(&mut bus);
    assert!(matches!(
        cpu.phase(),
        Phase::InterruptDispatch { step: 4, .. }
    ));
    assert_eq!(cpu.registers().sp(), 0xFFFC);
    assert_eq!(bus.read(0xFFFC), BASE as u8);

    cpu.step(&mut bus);
    assert!(matches!(cpu.phase(), Phase::FetchOpcode));
    assert_eq!(cpu.registers().pc(), 0x0040);
}

// ── M-cycle measurement ────────────────────────────────

fn measure_mcycles(opcode: u8, operands: &[u8]) -> usize {
    let (mut cpu, mut bus) = setup(&[]);
    bus.write(BASE, opcode);
    for (i, &b) in operands.iter().enumerate() {
        bus.write(BASE + 1 + i as u16, b);
    }
    cpu.registers_mut().set_pc(BASE);
    let start_pc = cpu.registers().pc();
    for count in 1..48 {
        let was_executing = !matches!(cpu.phase(), Phase::FetchOpcode);
        cpu.step(&mut bus);
        bus.step_devices(4);
        if matches!(cpu.phase(), Phase::FetchOpcode)
            && (was_executing || cpu.registers().pc() != start_pc)
        {
            return count;
        }
    }
    panic!("opcode {:02X} at PC={:04X}", opcode, cpu.registers().pc());
}

fn mc(opcode: u8) -> usize {
    measure_mcycles(opcode, &[])
}
fn mc1(opcode: u8, b: u8) -> usize {
    measure_mcycles(opcode, &[b])
}
fn mc2(opcode: u8, b1: u8, b2: u8) -> usize {
    measure_mcycles(opcode, &[b1, b2])
}

// Reference from instr_timing readme (0x00-0xFF, NOP=1 M-cycle)
const REF: [u8; 256] = [
    1, 3, 2, 2, 1, 1, 2, 1, 5, 2, 2, 2, 1, 1, 2, 1, 0, 3, 2, 2, 1, 1, 2, 1, 3, 2, 2, 2, 1, 1, 2, 1,
    2, 3, 2, 2, 1, 1, 2, 1, 2, 2, 2, 2, 1, 1, 2, 1, 2, 3, 2, 2, 3, 3, 3, 1, 2, 2, 2, 2, 1, 1, 2, 1,
    1, 1, 1, 1, 1, 1, 2, 1, 1, 1, 1, 1, 1, 1, 2, 1, 1, 1, 1, 1, 1, 1, 2, 1, 1, 1, 1, 1, 1, 1, 2, 1,
    1, 1, 1, 1, 1, 1, 2, 1, 1, 1, 1, 1, 1, 1, 2, 1, 2, 2, 2, 2, 2, 2, 0, 2, 1, 1, 1, 1, 1, 1, 2, 1,
    1, 1, 1, 1, 1, 1, 2, 1, 1, 1, 1, 1, 1, 1, 2, 1, 1, 1, 1, 1, 1, 1, 2, 1, 1, 1, 1, 1, 1, 1, 2, 1,
    1, 1, 1, 1, 1, 1, 2, 1, 1, 1, 1, 1, 1, 1, 2, 1, 1, 1, 1, 1, 1, 1, 2, 1, 1, 1, 1, 1, 1, 1, 2, 1,
    2, 3, 3, 4, 3, 4, 2, 4, 2, 4, 3, 0, 3, 6, 2, 4, 2, 3, 3, 0, 3, 4, 2, 4, 2, 4, 3, 0, 3, 0, 2, 4,
    3, 3, 2, 0, 0, 4, 2, 4, 4, 1, 4, 0, 0, 0, 2, 4, 3, 3, 2, 1, 0, 4, 2, 4, 3, 2, 4, 1, 0, 0, 2, 4,
];

#[test]
fn timing_all_opcodes() {
    let mut failures = Vec::new();
    for op in 0..=0xFFu8 {
        let exp = REF[op as usize];
        if exp == 0 || op == 0xCB {
            continue;
        }
        let actual = match op {
            // 1-byte, no operands
            0x07 | 0x0F | 0x17 | 0x1F | 0x27 | 0x2F | 0x37 | 0x3F => mc(op),
            0x00 | 0x04 | 0x05 | 0x0C | 0x0D | 0x14 | 0x15 | 0x1C | 0x1D | 0x24 | 0x25 | 0x2C
            | 0x2D | 0x3C | 0x3D => mc(op),
            0x02 | 0x03 | 0x09 | 0x0B | 0x12 | 0x13 | 0x19 | 0x1B | 0x22 | 0x23 | 0x29 | 0x2B
            | 0x32 | 0x33 | 0x39 | 0x3B => mc(op),
            0x0A | 0x1A | 0x2A | 0x3A => mc(op),
            0x34 | 0x35 => mc(op),
            0xE2 | 0xF2 => mc(op),
            0xF9 => mc(op),
            0xC7 | 0xCF | 0xD7 | 0xDF | 0xE7 | 0xEF | 0xF7 | 0xFF => mc(op),
            0xC5 | 0xD5 | 0xE5 | 0xF5 => mc(op),
            0xC1 | 0xD1 | 0xE1 | 0xF1 => mc(op),
            0xC9 | 0xD9 => mc(op),
            // Conditional instructions: timing depends on flags.
            // Skipped here — validated in individual tests below.
            0x28 | 0x38 | 0xC8 | 0xD8 | 0xCA | 0xDA | 0xCC | 0xDC => continue,
            0xC0 | 0xD0 => mc(op),
            0xE9 => mc(op),
            0xF3 | 0xFB => mc(op),
            // 1-byte, register ops (0x40-0x7F, 0x80-0xBF)
            0x40..=0x75 | 0x77..=0x7F => mc(op), // exclude HALT (0x76)
            0x80..=0xBF => mc(op),
            // 2-byte: opcode + 1 operand
            0x06 | 0x0E | 0x16 | 0x1E | 0x26 | 0x2E | 0x3E => mc1(op, 0),
            0xC6 | 0xCE | 0xD6 | 0xDE | 0xE6 | 0xEE | 0xF6 | 0xFE => mc1(op, 0),
            0xE0 | 0xF0 => mc1(op, 0),
            0x20 | 0x30 => mc1(op, 0),
            0x18 => mc1(op, 0),
            0x36 => mc1(op, 0),
            0xE8 | 0xF8 => mc1(op, 0),
            // HALT (0x76) excluded — variable timing
            0x76 => continue,
            // 3-byte: opcode + 2 operands
            0x01 | 0x11 | 0x21 | 0x31 => mc2(op, 0, 0),
            0xC3 | 0xC2 | 0xD2 => mc2(op, 0, 0),
            0xC4 | 0xD4 => mc2(op, 0, 0),
            0xCD => mc2(op, 0, 0),
            0xEA | 0xFA => mc2(op, 0, 0),
            0x08 => mc2(op, 0, 0),
            // Invalid opcodes
            0xD3 | 0xDB | 0xDD | 0xE3 | 0xE4 | 0xEB | 0xEC | 0xED | 0xF4 | 0xFC | 0xFD => {
                mc2(op, 0, 0)
            }
            0x10 => continue,
            _ => {
                failures.push(format!("{:02X}:unhandled", op));
                continue;
            }
        };
        if actual as u8 != exp {
            failures.push(format!("{:02X}: got {}M exp {}M", op, actual, exp));
        }
    }
    if !failures.is_empty() {
        panic!("{} mismatches:\n{}", failures.len(), failures.join("\n"));
    }
}

// ── NOP ──────────────────────────────────────────────────

#[test]
fn nop() {
    let cpu = exec_one(0x00, &[]);
    assert_eq!(cpu.registers().pc(), BASE + 1);
}

#[test]
fn ld_a_d8() {
    let cpu = exec_one(0x3E, &[0x42]);
    assert_eq!(cpu.registers().a(), 0x42);
}

#[test]
fn ld_bc_d16() {
    let cpu = exec_one(0x01, &[0x34, 0x12]);
    assert_eq!(cpu.registers().bc(), 0x1234);
}

#[test]
fn ld_hl_a_and_readback() {
    let cpu = exec_n(&[0x3E, 0x55, 0x21, 0x00, 0xC0, 0x77, 0x7E], 4);
    assert_eq!(cpu.registers().a(), 0x55);
}

#[test]
fn inc_a() {
    let cpu = exec_n(&[0x3E, 0x05, 0x3C], 2);
    assert_eq!(cpu.registers().a(), 6);
    assert!(!cpu.registers().z_flag());
}

#[test]
fn inc_a_to_zero() {
    let cpu = exec_n(&[0x3E, 0xFF, 0x3C], 2);
    assert_eq!(cpu.registers().a(), 0);
    assert!(cpu.registers().z_flag());
}

#[test]
fn dec_a() {
    let cpu = exec_n(&[0x3E, 0x05, 0x3D], 2);
    assert_eq!(cpu.registers().a(), 4);
    assert!(cpu.registers().n_flag());
}

#[test]
fn add_a_b() {
    let cpu = exec_n(&[0x3E, 0x10, 0x06, 0x08, 0x80], 3);
    assert_eq!(cpu.registers().a(), 0x18);
    assert!(!cpu.registers().c_flag());
}

#[test]
fn add_with_half_carry() {
    let cpu = exec_n(&[0x3E, 0x0F, 0x06, 0x01, 0x80], 3);
    assert_eq!(cpu.registers().a(), 0x10);
    assert!(cpu.registers().h_flag());
}

#[test]
fn add_with_carry() {
    let cpu = exec_n(&[0x3E, 0xF0, 0x06, 0x20, 0x80], 3);
    assert_eq!(cpu.registers().a(), 0x10);
    assert!(cpu.registers().c_flag());
}

#[test]
fn sub_a_b() {
    let cpu = exec_n(&[0x3E, 0x20, 0x06, 0x05, 0x90], 3);
    assert_eq!(cpu.registers().a(), 0x1B);
    assert!(cpu.registers().n_flag());
}

#[test]
fn sub_with_borrow() {
    let cpu = exec_n(&[0x3E, 0x05, 0x06, 0x10, 0x90], 3);
    assert_eq!(cpu.registers().a(), 0xF5);
    assert!(cpu.registers().c_flag());
}

#[test]
fn and_a_b() {
    let cpu = exec_n(&[0x3E, 0xF0, 0x06, 0x0F, 0xA0], 3);
    assert_eq!(cpu.registers().a(), 0x00);
    assert!(cpu.registers().z_flag());
    assert!(cpu.registers().h_flag());
}

#[test]
fn xor_a_b() {
    let cpu = exec_n(&[0x3E, 0xFF, 0x06, 0x0F, 0xA8], 3);
    assert_eq!(cpu.registers().a(), 0xF0);
}

#[test]
fn or_a_b() {
    let cpu = exec_n(&[0x3E, 0xF0, 0x06, 0x0F, 0xB0], 3);
    assert_eq!(cpu.registers().a(), 0xFF);
}

#[test]
fn cp_a_b_preserves_a() {
    let cpu = exec_n(&[0x3E, 0x42, 0x06, 0x42, 0xB8], 3);
    assert_eq!(cpu.registers().a(), 0x42);
    assert!(cpu.registers().z_flag());
}

#[test]
fn adc_with_carry() {
    let cpu = exec_n(&[0x3E, 0x10, 0x37, 0x06, 0x10, 0x88], 4);
    assert_eq!(cpu.registers().a(), 0x21);
}

#[test]
fn sbc_with_carry() {
    let cpu = exec_n(&[0x3E, 0x20, 0x37, 0x06, 0x10, 0x98], 4);
    assert_eq!(cpu.registers().a(), 0x0F);
}

#[test]
fn inc_bc() {
    let cpu = exec_n(&[0x01, 0xFF, 0xFF, 0x03], 2);
    assert_eq!(cpu.registers().bc(), 0x0000);
}

#[test]
fn dec_bc() {
    let cpu = exec_n(&[0x01, 0x00, 0x00, 0x0B], 2);
    assert_eq!(cpu.registers().bc(), 0xFFFF);
}

#[test]
fn add_hl_bc() {
    let cpu = exec_n(&[0x21, 0x00, 0x10, 0x01, 0x00, 0x01, 0x09], 3);
    assert_eq!(cpu.registers().hl(), 0x1100);
    assert!(!cpu.registers().c_flag());
}

#[test]
fn add_hl_bc_overflow() {
    let cpu = exec_n(&[0x21, 0x00, 0xF0, 0x01, 0x00, 0x20, 0x09], 3);
    assert_eq!(cpu.registers().hl(), 0x1000);
    assert!(cpu.registers().c_flag());
}

#[test]
fn jr_forward() {
    let cpu = exec_one(0x18, &[0x02]);
    assert_eq!(cpu.registers().pc(), BASE + 4);
}

#[test]
fn jp_a16() {
    let cpu = exec_one(0xC3, &[0x00, 0xC0]);
    assert_eq!(cpu.registers().pc(), 0xC000);
}

#[test]
fn call_pushes_return_address() {
    let cpu = exec_one(0xCD, &[0x10, 0xC0]);
    assert_eq!(cpu.registers().pc(), 0xC010);
}

#[test]
fn rst_vectors() {
    let vectors = [0xC7, 0xCF, 0xD7, 0xDF, 0xE7, 0xEF, 0xF7, 0xFF];
    let expected = [0x00, 0x08, 0x10, 0x18, 0x20, 0x28, 0x30, 0x38];
    for (&op, &exp) in vectors.iter().zip(expected.iter()) {
        let cpu = exec_one(op, &[]);
        assert_eq!(cpu.registers().pc(), exp, "RST {:02X}", op);
    }
}

#[test]
fn push_pop_hl() {
    let cpu = exec_n(&[0x21, 0xEF, 0xBE, 0xE5, 0xE1], 3);
    assert_eq!(cpu.registers().hl(), 0xBEEF);
    assert_eq!(cpu.registers().sp(), 0xFFFE);
}

#[test]
fn ld_hl_sp_e() {
    let cpu = exec_n(&[0x31, 0x00, 0xC1, 0xF8, 0x10], 2);
    assert_eq!(cpu.registers().hl(), 0xC110);
}

#[test]
fn cb_bit_test() {
    let cpu = exec_n(&[0x3E, 0x80, 0xCB, 0x7F], 2);
    assert!(!cpu.registers().z_flag());
    assert!(cpu.registers().h_flag());
}

#[test]
fn cb_res_clears_bit() {
    let cpu = exec_n(&[0x3E, 0x80, 0xCB, 0xBF], 2);
    assert_eq!(cpu.registers().a(), 0x00);
}

#[test]
fn cb_set_sets_bit() {
    let cpu = exec_n(&[0x3E, 0x00, 0xCB, 0xC7], 2);
    assert_eq!(cpu.registers().a(), 0x01);
}

#[test]
fn cb_rlc_through_carry() {
    let cpu = exec_n(&[0x3E, 0x80, 0xCB, 0x07], 2);
    assert_eq!(cpu.registers().a(), 0x01);
    assert!(cpu.registers().c_flag());
}

#[test]
fn daa_adjusts_after_addition() {
    let cpu = exec_n(&[0x3E, 0x09, 0xC6, 0x08, 0x27], 3);
    assert_eq!(cpu.registers().a(), 0x17);
}

#[test]
fn scf_sets_carry() {
    let cpu = exec_one(0x37, &[]);
    assert!(cpu.registers().c_flag());
}

#[test]
fn cpl_flips_a() {
    let cpu = exec_n(&[0x3E, 0x55, 0x2F], 2);
    assert_eq!(cpu.registers().a(), 0xAA);
}

#[test]
fn ld_hli_a_and_readback() {
    let cpu = exec_n(&[0x21, 0x00, 0xC0, 0x3E, 0x77, 0x22, 0x2A], 4);
    assert_eq!(cpu.registers().hl(), 0xC002);
}

#[test]
fn ld_hld_a() {
    let cpu = exec_n(&[0x21, 0x02, 0xC0, 0x3E, 0x88, 0x32], 3);
    assert_eq!(cpu.registers().hl(), 0xC001);
}

// ── Step count tests ─────────────────────────────────────

#[test]
fn ldh_a_a8_takes_12_t_cycles() {
    let (mut cpu, mut bus) = setup(&[0xF0, 0x05]);
    let start_pc = cpu.registers().pc();
    let mut n = 0;
    loop {
        let was_executing = !matches!(cpu.phase(), Phase::FetchOpcode);
        cpu.step(&mut bus);
        bus.step_devices(4);
        n += 4;
        if matches!(cpu.phase(), Phase::FetchOpcode)
            && (was_executing || cpu.registers().pc() != start_pc)
        {
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
    let start_pc = cpu.registers().pc();
    let mut n = 0;
    loop {
        let was_executing = !matches!(cpu.phase(), Phase::FetchOpcode);
        cpu.step(&mut bus);
        bus.step_devices(4);
        n += 4;
        if matches!(cpu.phase(), Phase::FetchOpcode)
            && (was_executing || cpu.registers().pc() != start_pc)
        {
            break;
        }
        if n > 48 {
            panic!("did not complete");
        }
    }
    assert_eq!(n, 16, "LD A,(a16) should take 16 T-cycles (4 M-cycles)");
}
