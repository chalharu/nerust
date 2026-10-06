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

use nerust_core_traits::debugger::{SpaceAccess, SpaceId, SpaceInfo, SpaceTable};

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
#[allow(dead_code)]
static GBC_SPACE_TABLE: SpaceTable = SpaceTable::build(&GBC_SPACES);

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
}
