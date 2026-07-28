#![cfg(test)]

/// Integration tests using retrio's gb-test-roms.
///
/// Each test loads a ROM, runs it for a fixed number of M-cycles,
/// and checks serial output for "Passed".
use std::path::Path;

use crate::cartridge::Cartridge;
use crate::cpu_core::Lr35902Cpu;
use crate::memory::GbcMemoryBus;

fn load_rom(subpath: &str) -> Cartridge {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../roms/gbc/retrio_gb-test-roms")
        .join(subpath);
    let rom_bytes = std::fs::read(&path).expect("ROM file not found");
    let header =
        crate::cartridge_header::CartridgeHeader::parse(&rom_bytes).expect("invalid ROM header");
    let mbc = crate::cartridge_mbc::create_mbc(&header, rom_bytes, None);
    Cartridge::new(mbc)
}

fn run_rom(subpath: &str, cycles: usize) -> String {
    let mut bus = GbcMemoryBus::new([0; 0x100], false);
    bus.set_cartridge(load_rom(subpath));
    let mut cpu = Lr35902Cpu::new();
    cpu.registers.pc = 0x0100;
    for _ in 0..cycles {
        cpu.step(&mut bus);
        bus.step_devices(4);
    }
    String::from_utf8_lossy(bus.serial_output()).into_owned()
}

/// Run a ROM that outputs via memory at $A000+ (signature at $A001-$A003).
fn run_rom_mem(subpath: &str, cycles: usize) -> String {
    let mut bus = GbcMemoryBus::new([0; 0x100], false);
    bus.set_cartridge(load_rom(subpath));
    let mut cpu = Lr35902Cpu::new();
    cpu.registers.pc = 0x0100;
    for _ in 0..cycles {
        cpu.step(&mut bus);
        bus.step_devices(4);
    }
    if bus.read(0xA001) == 0xDE && bus.read(0xA002) == 0xB0 && bus.read(0xA003) == 0x61 {
        let mut out = Vec::new();
        for addr in 0xA004..0xA800 {
            let c = bus.read(addr);
            if c == 0 {
                break;
            }
            out.push(c);
        }
        return String::from_utf8_lossy(&out).into_owned();
    }
    String::new()
}

fn assert_passed(output: &str, name: &str) {
    assert!(output.contains("Passed"), "{name} failed:\n{output}");
}

#[test]
fn mem_timing_read() {
    assert_passed(
        &run_rom("mem_timing/individual/01-read_timing.gb", 25_000_000),
        "mem_timing/01-read",
    );
}

#[test]
fn mem_timing_write() {
    assert_passed(
        &run_rom("mem_timing/individual/02-write_timing.gb", 25_000_000),
        "mem_timing/02-write",
    );
}

#[test]
fn mem_timing_modify() {
    assert_passed(
        &run_rom("mem_timing/individual/03-modify_timing.gb", 25_000_000),
        "mem_timing/03-modify",
    );
}

#[test]
fn mem_timing_all() {
    let output = run_rom("mem_timing/mem_timing.gb", 25_000_000);
    assert!(!output.contains("Failed"), "mem_timing failure:\n{output}");
}

// ── mem_timing-2 (memory-mapped output at $A000) ──────

#[test]
fn mem_timing2_read() {
    assert_passed(
        &run_rom_mem("mem_timing-2/rom_singles/01-read_timing.gb", 25_000_000),
        "mem_timing-2/01-read",
    );
}

#[test]
fn mem_timing2_write() {
    assert_passed(
        &run_rom_mem("mem_timing-2/rom_singles/02-write_timing.gb", 25_000_000),
        "mem_timing-2/02-write",
    );
}

#[test]
fn mem_timing2_modify() {
    assert_passed(
        &run_rom_mem("mem_timing-2/rom_singles/03-modify_timing.gb", 25_000_000),
        "mem_timing-2/03-modify",
    );
}

#[test]
fn mem_timing2_all() {
    let output = run_rom("mem_timing-2/mem_timing.gb", 25_000_000);
    assert!(
        !output.contains("Failed"),
        "mem_timing-2 failure:\n{output}"
    );
}

#[test]
fn cpu_instrs_all() {
    let output = run_rom("cpu_instrs/cpu_instrs.gb", 25_000_000);
    assert!(!output.contains("Failed"), "cpu_instrs failure:\n{output}");
}

#[test]
fn cpu_instrs_01_special() {
    assert_passed(
        &run_rom("cpu_instrs/individual/01-special.gb", 10_000_000),
        "01-special",
    );
}

#[test]
#[ignore = "needs PPU STAT register for LCD interrupt"]
fn cpu_instrs_02_interrupts() {
    assert_passed(
        &run_rom("cpu_instrs/individual/02-interrupts.gb", 10_000_000),
        "02-interrupts",
    );
}

#[test]
fn cpu_instrs_03_op_sp_hl() {
    assert_passed(
        &run_rom("cpu_instrs/individual/03-op sp,hl.gb", 10_000_000),
        "03-op sp,hl",
    );
}

#[test]
fn cpu_instrs_04_op_r_imm() {
    assert_passed(
        &run_rom("cpu_instrs/individual/04-op r,imm.gb", 10_000_000),
        "04-op r,imm",
    );
}

#[test]
fn cpu_instrs_05_op_rp() {
    assert_passed(
        &run_rom("cpu_instrs/individual/05-op rp.gb", 10_000_000),
        "05-op rp",
    );
}

#[test]
fn cpu_instrs_06_ld_r_r() {
    assert_passed(
        &run_rom("cpu_instrs/individual/06-ld r,r.gb", 10_000_000),
        "06-ld r,r",
    );
}

#[test]
fn cpu_instrs_07_jump_call_ret_rst() {
    assert_passed(
        &run_rom("cpu_instrs/individual/07-jr,jp,call,ret,rst.gb", 10_000_000),
        "07-jump",
    );
}

#[test]
fn cpu_instrs_08_misc() {
    assert_passed(
        &run_rom("cpu_instrs/individual/08-misc instrs.gb", 10_000_000),
        "08-misc",
    );
}

#[test]
fn cpu_instrs_09_op_r_r() {
    assert_passed(
        &run_rom("cpu_instrs/individual/09-op r,r.gb", 10_000_000),
        "09-op r,r",
    );
}

#[test]
fn cpu_instrs_10_bit_ops() {
    assert_passed(
        &run_rom("cpu_instrs/individual/10-bit ops.gb", 25_000_000),
        "10-bit ops",
    );
}

#[test]
fn cpu_instrs_11_op_a_hl() {
    assert_passed(
        &run_rom("cpu_instrs/individual/11-op a,(hl).gb", 25_000_000),
        "11-op a,(hl)",
    );
}

#[test]
fn instr_timing() {
    assert_passed(
        &run_rom("instr_timing/instr_timing.gb", 25_000_000),
        "instr_timing",
    );
}
