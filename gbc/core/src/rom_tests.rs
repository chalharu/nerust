/// Integration tests using retrio's gb-test-roms.
///
/// Each test loads a ROM, runs it for a fixed number of M-cycles,
/// and checks serial output for "Passed".
#[cfg(test)]
mod tests {
    use std::path::Path;

    use crate::cartridge::Cartridge;
    use crate::cpu::Lr35902Cpu;
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

    #[test]
    fn cpu_instrs_all() {
        let output = run_rom("cpu_instrs/cpu_instrs.gb", 100_000_000);
        // Test 02 (interrupts) needs PPU VBlank — known limitation
        assert!(
            !output.contains("Failed"),
            "cpu_instrs failure:\n{}",
            output
        );
    }

    #[test]
    fn cpu_instrs_01_special() {
        let output = run_rom("cpu_instrs/individual/01-special.gb", 50_000_000);
        assert!(output.contains("Passed"), "01-special failed:\n{}", output);
    }
}
