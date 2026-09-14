//! Temporary probe. DELETE BEFORE COMMIT.
use nerust_gba_core::system::GbaSystem;

#[test]
fn probe_blr14c() {
    let path = "../../roms/gba/alyosha_gba-tests/irq/BL_IRQ_R14.gba";
    let rom = std::fs::read(path).unwrap();
    let mut sys = GbaSystem::from_test_rom(rom).unwrap();
    for cycle in 0..3_000_000 {
        sys.step_tcycle();
        let pc = sys.cpu.registers().pc();
        if pc == 0x08000180 || pc == 0x08000184 {
            eprintln!("cycle {cycle}: pc={pc:#x} r0={:#x} mode={:#x} r14={:#x}",
                sys.cpu.registers().r(0), sys.cpu.registers().cpsr() & 0x1F,
                sys.cpu.registers().r(14));
        }
        if cycle == 2_999_999 {
            eprintln!("done r12={:#x}", sys.cpu.registers().r(12));
        }
    }
}
