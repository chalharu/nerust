#![cfg(test)]

/// Integration tests using retrio's gb-test-roms.
///
/// Each test loads a ROM, runs it for a fixed number of M-cycles,
/// and checks serial output for "Passed".
use std::path::Path;

use crate::cartridge::Cartridge;
use crate::cpu_core::Lr35902Cpu;
use crate::memory::{CpuStepper, GbcMemoryBus};

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
    cpu.registers_mut().set_pc(0x0100);
    for _ in 0..cycles {
        cpu.step(&mut bus);
        bus.step_devices(4);
    }
    String::from_utf8_lossy(bus.serial_output()).into_owned()
}

/// Run a ROM that outputs via memory at $A000+ (signature at $A001-$A003).
fn run_rom_mem_for_model(subpath: &str, cycles: usize, cgb: bool) -> String {
    let mut bus = GbcMemoryBus::new([0; 0x100], false);
    bus.set_cgb_mode(cgb);
    bus.set_cartridge(load_rom(subpath));
    let mut cpu = Lr35902Cpu::new();
    cpu.registers_mut().set_pc(0x0100);
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

fn run_rom_mem(subpath: &str, cycles: usize) -> String {
    run_rom_mem_for_model(subpath, cycles, false)
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
fn cpu_instrs_02_interrupts() {
    assert_passed(
        &run_rom("cpu_instrs/individual/02-interrupts.gb", 25_000_000),
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

#[test]
#[ignore = "frame buffer hash verification not yet implemented; renders reference image via PPU"]
fn dmg_acid2() {
    let output = run_rom("mattcurrie_dmg-acid2/dmg-acid2.gb", 10_000_000);
    // dmg-acid2 does not output via serial; verification requires frame buffer hash
    // Currently just verify the ROM runs without panicking
    assert!(
        output.is_empty() || output.contains("Passed"),
        "dmg-acid2 should not produce serial output"
    );
}

#[test]
fn halt_bug() {
    assert_passed(&run_rom_mem("halt_bug.gb", 10_000_000), "halt_bug");
}

#[test]
#[ignore = "OAM DMA bug not implemented"]
fn oam_bug_1_lcd_sync() {
    assert_passed(
        &run_rom_mem("oam_bug/rom_singles/1-lcd_sync.gb", 10_000_000),
        "oam_bug/1-lcd_sync",
    );
}

#[test]
#[ignore = "OAM DMA corruption bug not implemented"]
fn oam_bug_2_causes() {
    assert_passed(
        &run_rom_mem("oam_bug/rom_singles/2-causes.gb", 10_000_000),
        "oam_bug_2",
    );
}

#[test]
#[ignore = "OAM DMA corruption bug not implemented"]
fn oam_bug_3_non_causes() {
    assert_passed(
        &run_rom_mem("oam_bug/rom_singles/3-non_causes.gb", 10_000_000),
        "oam_bug_3",
    );
}

#[test]
#[ignore = "OAM DMA corruption bug not implemented"]
fn oam_bug_4_scanline_timing() {
    assert_passed(
        &run_rom_mem("oam_bug/rom_singles/4-scanline_timing.gb", 10_000_000),
        "oam_bug_4",
    );
}

#[test]
#[ignore = "OAM DMA corruption bug not implemented"]
fn oam_bug_5_timing_bug() {
    assert_passed(
        &run_rom_mem("oam_bug/rom_singles/5-timing_bug.gb", 10_000_000),
        "oam_bug_5",
    );
}

#[test]
#[ignore = "OAM DMA corruption bug not implemented"]
fn oam_bug_6_timing_no_bug() {
    assert_passed(
        &run_rom_mem("oam_bug/rom_singles/6-timing_no_bug.gb", 10_000_000),
        "oam_bug_6",
    );
}

#[test]
#[ignore = "OAM DMA corruption bug not implemented"]
fn oam_bug_7_timing_effect() {
    assert_passed(
        &run_rom_mem("oam_bug/rom_singles/7-timing_effect.gb", 10_000_000),
        "oam_bug_7",
    );
}

#[test]
#[ignore = "OAM DMA corruption bug not implemented"]
fn oam_bug_8_instr_effect() {
    assert_passed(
        &run_rom_mem("oam_bug/rom_singles/8-instr_effect.gb", 10_000_000),
        "oam_bug_8",
    );
}

fn assert_sound_passed(suite: &str, rom: &str, cgb: bool) {
    let path = format!("{suite}/rom_singles/{rom}.gb");
    assert_passed(&run_rom_mem_for_model(&path, 60_000_000, cgb), &path);
}

macro_rules! sound_tests {
    ($module:ident, $suite:literal, $cgb:literal, $wave_test:literal) => {
        mod $module {
            use super::*;

            #[test]
            fn combined() {
                let path = concat!($suite, "/", $suite, ".gb");
                assert_passed(&run_rom_mem_for_model(path, 60_000_000, $cgb), path);
            }

            macro_rules! sound_test {
                ($name:ident, $rom:literal) => {
                    #[test]
                    fn $name() {
                        assert_sound_passed($suite, $rom, $cgb);
                    }
                };
            }

            sound_test!(registers, "01-registers");
            sound_test!(length_counter, "02-len ctr");
            sound_test!(trigger, "03-trigger");
            sound_test!(sweep, "04-sweep");
            sound_test!(sweep_details, "05-sweep details");
            sound_test!(overflow_on_trigger, "06-overflow on trigger");
            sound_test!(length_sweep_period_sync, "07-len sweep period sync");
            sound_test!(length_counter_during_power, "08-len ctr during power");
            sound_test!(wave_read_while_on, "09-wave read while on");
            sound_test!(wave_trigger_while_on, "10-wave trigger while on");
            sound_test!(registers_after_power, "11-regs after power");
            sound_test!(wave_write_while_on, $wave_test);
        }
    };
}

sound_tests!(dmg_sound, "dmg_sound", false, "12-wave write while on");
sound_tests!(cgb_sound, "cgb_sound", true, "12-wave");
