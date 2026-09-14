//! Temporary probe. DELETE BEFORE COMMIT.
use nerust_gba_core::system::GbaSystem;

#[test]
fn probe_latch() {
    let path = "../../roms/gba/nba-emu_hw-test/dma/latch/latch.gba";
    let rom = std::fs::read(path).unwrap();
    let mut sys = GbaSystem::from_test_rom(rom).unwrap();
    for _ in 0..3_000_000 {
        sys.step_tcycle();
    }
    for a in [0x0300014Cu32, 0x03000150, 0x03000000, 0x03000004] {
        eprintln!("iwram {a:#x} = {:#x}", sys.bus.read32(a));
    }
}
