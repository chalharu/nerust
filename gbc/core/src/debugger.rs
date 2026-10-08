//! GBC debugger address space.
//!
//! Staged introduction: this module currently owns only the static
//! [`SpaceTable`]. The `Debugger`/`DebugControl` implementation that
//! reads through it lands next; the table is fixed first because every
//! read, write, and dump resolves against it.
//!
//! Deliberately a single space: regions are already distinguished by
//! address, so extra ids would buy nothing — the reader dispatches on
//! the address internally. The one trade-off is typo detection: gap
//! addresses resolve inside the space and read as `None` (`OpenBus`)
//! instead of failing loudly as `Unmapped`. Never assert `open_bus`
//! on a gap address.
//!
//! Reader contract (for the next step; all verified against
//! `memory.rs` decode):
//!
//! - Echo `0xE000..=0xFDFF` normalizes to `0xC000..=0xDDFF` (`-0x2000`).
//! - `0xFFFF` serves the IE register, not the HRAM array.
//! - The unusable gap `0xFEA0..=0xFEFF` reads `0x00` on hardware;
//!   return `Some(0x00)` (mapped), not `None`.
//! - `0xA000..=0xBFFF` reads `0xFF` when RAM is disabled or absent
//!   (the MBC exposes no driven/floating mask): byte-exact, but
//!   `open_bus` assertions are meaningless there.
//! - CGB-only registers return their fixed DMG values; VRAM reads go
//!   through `debug_read_vram` (no mode locks); OAM is readable —
//!   frame steps always park in VBlank, so the value is deterministic.
//! - All sub-reads are `&self` with no mutation (audited: APU, PPU,
//!   serial, timer, interrupts, joypad).

use std::cell::OnceCell;

use nerust_core_traits::debugger::{Debugger, SpaceAccess, SpaceId, SpaceInfo, SpaceTable};

use crate::system::GbcSystem;

/// Whole 16-bit bus (`0x0000..=0xFFFF`). The only GBC space.
pub const SPACE_MEMORY: SpaceId = SpaceId(0);

static GBC_SPACES: [SpaceInfo; 1] = [SpaceInfo {
    id: SPACE_MEMORY,
    key: "memory",
    name: "Memory",
    address_bits: 16,
    range: 0x0000..=0xFFFF,
    access: SpaceAccess::ReadOnly,
}];

/// Validated GBC memory-space table.
pub(crate) static GBC_SPACE_TABLE: SpaceTable = SpaceTable::build(&GBC_SPACES);

/// Read-only GBC observer.
///
/// Built per use (built, read, dropped); `registers()` borrows a buffer
/// filled lazily on first use. In phase 1 every decoded address is driven —
/// there are no floating reads — so `read` is `Some` for the whole
/// range and `None` only outside it.
pub struct GbcDebugger<'a> {
    system: &'a GbcSystem,
    /// Lazily filled on first `registers()`: screen/memory-only
    /// asserts never pay for register collection.
    regs: OnceCell<Vec<(&'static str, u64)>>,
}

impl<'a> GbcDebugger<'a> {
    pub fn new(system: &'a GbcSystem) -> Self {
        Self {
            system,
            regs: OnceCell::new(),
        }
    }

    fn snapshot_registers(&self) -> Vec<(&'static str, u64)> {
        let regs = self.system.cpu.registers();
        Vec::from([
            ("a", u64::from(regs.a())),
            ("b", u64::from(regs.b())),
            ("c", u64::from(regs.c())),
            ("d", u64::from(regs.d())),
            ("e", u64::from(regs.e())),
            ("f", u64::from(regs.f())),
            ("h", u64::from(regs.h())),
            ("l", u64::from(regs.l())),
            ("pc", u64::from(regs.pc())),
            ("sp", u64::from(regs.sp())),
        ])
    }

    fn read_byte(&self, addr: u32) -> Option<u64> {
        Some(u64::from(self.system.bus.debug_read(addr as u16)))
    }
}

impl Debugger for GbcDebugger<'_> {
    fn spaces(&self) -> &[SpaceInfo] {
        &GBC_SPACES
    }

    fn space_containing(&self, addr: u32) -> Option<SpaceId> {
        GBC_SPACES
            .iter()
            .find(|s| s.range.contains(&addr))
            .map(|s| s.id)
    }

    fn read(&self, space: SpaceId, addr: u32, width: u8) -> Option<u64> {
        // Single gate: unknown SpaceId, out-of-range, range-crossing,
        // and bad widths (including 0) all become None here. The cast
        // below is safe: covers() pins addr + width - 1 <= 0xFFFF.
        if !GBC_SPACE_TABLE.covers(space, addr, width) {
            return None;
        }
        // Little-endian composition of physical bytes.
        let mut value = 0u64;
        for i in 0..width as u32 {
            value |= self.read_byte(addr + i)? << (8 * i);
        }
        Some(value)
    }

    fn registers(&self) -> &[(&'static str, u64)] {
        self.regs.get_or_init(|| self.snapshot_registers())
    }
}

#[cfg(test)]
mod tests {
    use nerust_core_traits::debugger::validate_spaces;

    use super::*;

    #[test]
    fn gbc_spaces_validate() {
        GBC_SPACE_TABLE.validate().expect("GBC spaces valid");
        assert_eq!(validate_spaces(&GBC_SPACES), Ok(()));
    }

    #[test]
    fn gbc_space_is_single_full_bus_range() {
        assert_eq!(GBC_SPACES.len(), 1);
        let space = &GBC_SPACES[0];
        assert_eq!(space.id, SPACE_MEMORY);
        assert_eq!(*space.range.start(), 0x0000);
        assert_eq!(*space.range.end(), 0xFFFF);
        // covers() is the read/write gate: the full bus resolves here;
        // anything wider is rejected.
        assert!(GBC_SPACE_TABLE.covers(SPACE_MEMORY, 0x0000, 1));
        assert!(GBC_SPACE_TABLE.covers(SPACE_MEMORY, 0xC000, 2));
        assert!(GBC_SPACE_TABLE.covers(SPACE_MEMORY, 0xE123, 1));
        assert!(GBC_SPACE_TABLE.covers(SPACE_MEMORY, 0xFEA0, 1));
        assert!(GBC_SPACE_TABLE.covers(SPACE_MEMORY, 0xFFFF, 1));
        assert!(GBC_SPACE_TABLE.covers(SPACE_MEMORY, 0xFFFE, 2));
        assert!(!GBC_SPACE_TABLE.covers(SPACE_MEMORY, 0xFFFF, 2));
        assert!(!GBC_SPACE_TABLE.covers(SPACE_MEMORY, 0xFFFF, 4));
        assert!(!GBC_SPACE_TABLE.covers(SPACE_MEMORY, 0x10000, 1));
        assert!(!GBC_SPACE_TABLE.covers(SpaceId(1), 0xC000, 1));
        assert!(!GBC_SPACE_TABLE.covers(SPACE_MEMORY, 0xC000, 0));
    }

    fn rom() -> Vec<u8> {
        let mut rom = vec![0; 0x8000];
        rom[0x0100] = 0x18; // JR -2
        rom[0x0101] = 0xFE;
        rom[0x0143] = 0x80;
        rom[0x0147] = 0;
        rom[0x0148] = 0;
        rom[0x0149] = 0;
        crate::cartridge_header::finalize_test_rom(&mut rom);
        rom
    }

    fn live_system() -> GbcSystem {
        // The debugger borrows the live system; tests build one the
        // same way load does.
        let rom = rom();
        let descriptor = crate::cartridge_descriptor::detect_cartridge(&rom).expect("header");
        GbcSystem::from_descriptor(
            crate::core_options::GbcCoreOptions::default().hardware_model,
            rom,
            &descriptor,
        )
        .expect("system builds")
    }

    #[test]
    fn gbc_debugger_can_be_boxed() {
        let system = live_system();
        let boxed: Box<dyn Debugger + '_> = Box::new(GbcDebugger::new(&system));
        assert_eq!(boxed.spaces().len(), 1);
        assert_eq!(boxed.space_containing(0xC000), Some(SPACE_MEMORY));
        assert_eq!(boxed.space_containing(0xFFFF), Some(SPACE_MEMORY));
        assert_eq!(boxed.space_containing(0x10000), None);
        // Registers: 10 entries in ascending name order (kernel contract).
        let names: Vec<&str> = boxed.registers().iter().map(|(n, _)| *n).collect();
        assert_eq!(
            names,
            vec!["a", "b", "c", "d", "e", "f", "h", "l", "pc", "sp"]
        );
        // Live read path: fresh WRAM is readable, and the echo
        // mirror observes the same byte.
        assert!(boxed.read(SPACE_MEMORY, 0xC000, 1).is_some());
        assert_eq!(
            boxed.read(SPACE_MEMORY, 0xC000, 1),
            boxed.read(SPACE_MEMORY, 0xE000, 1)
        );
        // Little-endian composition.
        let lo = boxed.read(SPACE_MEMORY, 0xC000, 1).unwrap();
        let wide = boxed.read(SPACE_MEMORY, 0xC000, 2).unwrap();
        assert_eq!(wide & 0xFF, lo);
        // Unusable gap reads its hardware value, not None.
        assert_eq!(boxed.read(SPACE_MEMORY, 0xFEA0, 1), Some(0x00));
        // Rejected inputs collapse to None.
        assert_eq!(boxed.read(SPACE_MEMORY, 0xFFFF, 2), None);
        assert_eq!(boxed.read(SPACE_MEMORY, 0x10000, 1), None);
        assert_eq!(boxed.read(SPACE_MEMORY, 0xC000, 0), None);
        assert_eq!(boxed.read(SPACE_MEMORY, 0xC000, 3), None);
        assert_eq!(boxed.read(SpaceId(9), 0xC000, 1), None);
    }
}
