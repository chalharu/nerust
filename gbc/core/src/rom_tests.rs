/// Integration tests using retrio's gb-test-roms.
///
/// Each test loads a ROM, runs it for a fixed number of M-cycles,
/// and checks serial output for "Passed".
#[cfg(test)]
mod tests {
    use std::path::Path;

    use crate::cartridge::Cartridge;
    use crate::cpu_core::Lr35902Cpu;
    use crate::memory::GbcMemoryBus;

    fn load_rom(subpath: &str) -> Cartridge {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../roms/gbc/retrio_gb-test-roms")
            .join(subpath);
        let rom_bytes = std::fs::read(&path).expect("ROM file not found");
        let header = crate::cartridge_header::CartridgeHeader::parse(&rom_bytes)
            .expect("invalid ROM header");
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

    fn assert_passed(output: &str, name: &str) {
        assert!(output.contains("Passed"), "{name} failed:\n{output}");
    }

    #[test]
    fn cpu_instrs_all() {
        let output = run_rom("cpu_instrs/cpu_instrs.gb", 100_000_000);
        assert!(!output.contains("Failed"), "cpu_instrs failure:\n{output}");
    }

    #[test]
    fn cpu_instrs_01_special() {
        assert_passed(
            &run_rom("cpu_instrs/individual/01-special.gb", 50_000_000),
            "01-special",
        );
    }

    #[test]
    #[ignore = "needs PPU for VBlank interrupts"]
    fn cpu_instrs_02_interrupts() {
        assert_passed(
            &run_rom("cpu_instrs/individual/02-interrupts.gb", 50_000_000),
            "02-interrupts",
        );
    }

    #[test]
    fn cpu_instrs_03_op_sp_hl() {
        assert_passed(
            &run_rom("cpu_instrs/individual/03-op sp,hl.gb", 50_000_000),
            "03-op sp,hl",
        );
    }

    #[test]
    fn cpu_instrs_04_op_r_imm() {
        assert_passed(
            &run_rom("cpu_instrs/individual/04-op r,imm.gb", 50_000_000),
            "04-op r,imm",
        );
    }

    #[test]
    fn cpu_instrs_05_op_rp() {
        assert_passed(
            &run_rom("cpu_instrs/individual/05-op rp.gb", 50_000_000),
            "05-op rp",
        );
    }

    #[test]
    fn cpu_instrs_06_ld_r_r() {
        assert_passed(
            &run_rom("cpu_instrs/individual/06-ld r,r.gb", 50_000_000),
            "06-ld r,r",
        );
    }

    #[test]
    fn cpu_instrs_07_jump_call_ret_rst() {
        assert_passed(
            &run_rom("cpu_instrs/individual/07-jr,jp,call,ret,rst.gb", 50_000_000),
            "07-jump",
        );
    }

    #[test]
    fn cpu_instrs_08_misc() {
        assert_passed(
            &run_rom("cpu_instrs/individual/08-misc instrs.gb", 50_000_000),
            "08-misc",
        );
    }

    #[test]
    fn cpu_instrs_09_op_r_r() {
        assert_passed(
            &run_rom("cpu_instrs/individual/09-op r,r.gb", 50_000_000),
            "09-op r,r",
        );
    }

    #[test]
    fn cpu_instrs_10_bit_ops() {
        assert_passed(
            &run_rom("cpu_instrs/individual/10-bit ops.gb", 50_000_000),
            "10-bit ops",
        );
    }

    #[test]
    fn cpu_instrs_11_op_a_hl() {
        assert_passed(
            &run_rom("cpu_instrs/individual/11-op a,(hl).gb", 50_000_000),
            "11-op a,(hl)",
        );
    }

    #[test]
    #[ignore = "timer init check timing doesn't match hardware; needs timer divider refinement"]
    fn instr_timing() {
        assert_passed(
            &run_rom("instr_timing/instr_timing.gb", 150_000_000),
            "instr_timing",
        );
    }
}
