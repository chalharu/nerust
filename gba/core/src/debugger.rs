//! GBA debugger address space.
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
//! `memory/mod.rs` decode):
//!
//! - New `&self` peek only: the existing `read8/16/32` are `&mut` and
//!   charge wait cycles plus N/S-tracker progress. The region readers
//!   underneath (`read_ewram/iwram/palette/vram/oam/rom/sram`,
//!   `read_bios`) are already `&self` and reusable as-is.
//! - BIOS guard reproduced without mutation: protected reads serve the
//!   `bios_prefetch` latch (`&self` field read); only unprotected
//!   reads touch `read_bios`.
//! - OAM without the `cpu_bus` latch store that `read_mapped` performs.
//! - `0x0D` window: EEPROM cartridges serve `eeprom_peek_bit` (existing
//!   `&self` peek); plain ROMs mirror WS2 through `read_rom`. GPIO
//!   overlays apply in the same order as `read_rom`.
//! - SRAM answers only with a battery-backed chip: EEPROM cartridges
//!   and no-cartridge reads are `None` (honest open bus), never a
//!   fabricated `0xFF`.
//! - I/O (`0x04000000..=0x04000803`) serves the audited pure subset —
//!   PPU/APU/timers/key/IRQ fields, DMA CNT, constant holes — and
//!   returns `None` for the stateful SIO block (UART latch clear, FIFO
//!   pop), write-only registers, and gap lanes. Timer reads reuse the
//!   stamped cycle (no re-stamp, no mutation). Unmapped I/O is dynamic
//!   prefetch-latch open bus on hardware; the debugger does not
//!   reproduce it.
//! - Widths 2/4 compose physical bytes little-endian. CPU aligned-ROR
//!   semantics deliberately do not apply: the debugger shows physical
//!   contents.

use nerust_core_traits::debugger::{SpaceAccess, SpaceId, SpaceInfo, SpaceTable};

/// Whole decoded bus (`0x00000000..=0x0FFFFFFF`). The only GBA space.
/// Above `0x0FFFFFFF` stays `Unmapped` (loud manifest error).
pub const SPACE_MEMORY: SpaceId = SpaceId(0);

static GBA_SPACES: [SpaceInfo; 1] = [SpaceInfo {
    id: SPACE_MEMORY,
    key: "memory",
    name: "Memory",
    address_bits: 32,
    range: 0x00000000..=0x0FFFFFFF,
    access: SpaceAccess::ReadOnly,
}];

/// Validated GBA memory-space table.
#[allow(dead_code)]
static GBA_SPACE_TABLE: SpaceTable = SpaceTable::build(&GBA_SPACES);

#[cfg(test)]
mod tests {
    use nerust_core_traits::debugger::validate_spaces;

    use super::*;

    #[test]
    fn gba_spaces_validate() {
        GBA_SPACE_TABLE.validate().expect("GBA spaces valid");
        assert_eq!(validate_spaces(&GBA_SPACES), Ok(()));
    }

    #[test]
    fn gba_space_is_single_decoded_bus_range() {
        assert_eq!(GBA_SPACES.len(), 1);
        let space = &GBA_SPACES[0];
        assert_eq!(space.id, SPACE_MEMORY);
        assert_eq!(*space.range.start(), 0x00000000);
        assert_eq!(*space.range.end(), 0x0FFFFFFF);
        // covers() is the read/write gate: every decoded region
        // resolves here; beyond decode is rejected.
        assert!(GBA_SPACE_TABLE.covers(SPACE_MEMORY, 0x00000000, 4));
        assert!(GBA_SPACE_TABLE.covers(SPACE_MEMORY, 0x02000000, 1));
        assert!(GBA_SPACE_TABLE.covers(SPACE_MEMORY, 0x04000208, 2));
        assert!(GBA_SPACE_TABLE.covers(SPACE_MEMORY, 0x08000000, 4));
        assert!(GBA_SPACE_TABLE.covers(SPACE_MEMORY, 0x0D000000, 1));
        assert!(GBA_SPACE_TABLE.covers(SPACE_MEMORY, 0x0E000000, 1));
        assert!(GBA_SPACE_TABLE.covers(SPACE_MEMORY, 0x0FFFFFFF, 1));
        assert!(!GBA_SPACE_TABLE.covers(SPACE_MEMORY, 0x0FFFFFFF, 2));
        assert!(!GBA_SPACE_TABLE.covers(SPACE_MEMORY, 0x10000000, 1));
        assert!(!GBA_SPACE_TABLE.covers(SPACE_MEMORY, 0xFFFFFFFF, 4));
        assert!(!GBA_SPACE_TABLE.covers(SpaceId(1), 0x02000000, 1));
        assert!(!GBA_SPACE_TABLE.covers(SPACE_MEMORY, 0x02000000, 0));
    }
}
