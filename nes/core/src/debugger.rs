//! NES debugger: read-only observation plus controlled WRAM edits.
//!
//! Design: `debugger-trait-design.md` v7.0 §6.5. The observer is per-use
//! (built, read, dropped); `registers()` borrows a buffer filled once in
//! `new`, since `RefCell` cannot hand out borrows (E0515).

use std::cell::OnceCell;

use nerust_core_traits::debugger::{
    DebugImage, DebugPanel, Debugger, SpaceAccess, SpaceId, SpaceInfo, SpaceTable,
};

use crate::Core;

/// Work RAM with CPU mirrors ($0000-$07FF mirrored to $1FFF).
/// Matches `peek_work_ram` semantics exactly.
pub const SPACE_WORK_RAM: SpaceId = SpaceId(0);
/// PPU nametables and palette ($2000-$3EFF, $3F00-$3FFF).
/// CHR ($0000-$1FFF) is cartridge-dependent and intentionally excluded:
/// it has no stable range in this table.
pub const SPACE_PPU_VRAM: SpaceId = SpaceId(1);
/// Cartridge PRG RAM ($6000-$7FFF). Reads go through the bus-state
/// preserving peek: fully-driven bytes read as values, floating bytes
/// read as open (None). Absent from older tables — resolvers must use
/// the table itself, never positions.
pub const SPACE_CARTRIDGE_RAM: SpaceId = SpaceId(2);

static NES_SPACES: [SpaceInfo; 3] = [
    SpaceInfo {
        id: SPACE_WORK_RAM,
        key: "wram",
        name: "WRAM",
        address_bits: 13,
        range: 0x0000..=0x1FFF,
        access: SpaceAccess::ReadWrite,
    },
    SpaceInfo {
        id: SPACE_PPU_VRAM,
        key: "ppu_vram",
        name: "PPU VRAM",
        address_bits: 14,
        range: 0x2000..=0x3FFF,
        // No VRAM poke exists yet, so edits are refused for now.
        access: SpaceAccess::ReadOnly,
    },
    SpaceInfo {
        id: SPACE_CARTRIDGE_RAM,
        key: "cartridge_ram",
        name: "Cartridge RAM",
        address_bits: 13,
        range: 0x6000..=0x7FFF,
        // Pokes target WRAM only; cartridge bytes are read-only here.
        access: SpaceAccess::ReadOnly,
    },
];

/// Validated NES memory-space table.
pub(crate) static NES_SPACE_TABLE: SpaceTable = SpaceTable::build(&NES_SPACES);

/// Read-only NES observer.
pub struct NesDebugger<'a> {
    core: &'a Core,
    /// `registers()` backing buffer, filled on first use. The observer
    /// is a snapshot, and must not go stale mid-frame, so callers
    /// rebuild it per use instead of holding it across frames.
    /// Laziness is pay-for-what-you-use: screen/memory-only asserts
    /// never pay for register collection.
    regs: OnceCell<Vec<(&'static str, u64)>>,
}

impl<'a> NesDebugger<'a> {
    pub fn new(core: &'a Core) -> Self {
        Self {
            core,
            regs: OnceCell::new(),
        }
    }

    fn read_byte(&self, addr: u32) -> Option<u64> {
        self.core.peek_work_ram(addr as usize).map(u64::from)
    }

    fn read_vram_byte(&self, addr: u32) -> Option<u64> {
        self.core.peek_ppu_vram(addr as usize).map(u64::from)
    }

    /// Cartridge byte with bus state: fully-driven reads as a value,
    /// floating reads as open (None). `None` also covers absent RAM,
    /// so callers distinguish open from unmapped through table
    /// coverage, never through positions.
    fn read_cartridge_byte(&self, addr: u32) -> Option<u64> {
        self.core
            .peek_cartridge_ram(addr as usize)
            .and_then(|read| (read.mask == 0xFF).then(|| u64::from(read.data)))
    }
}

impl Debugger for NesDebugger<'_> {
    fn spaces(&self) -> &[SpaceInfo] {
        &NES_SPACES
    }

    fn space_containing(&self, addr: u32) -> Option<SpaceId> {
        NES_SPACES
            .iter()
            .find(|s| s.range.contains(&addr))
            .map(|s| s.id)
    }

    fn read(&self, space: SpaceId, addr: u32, width: u8) -> Option<u64> {
        // Single gate: unknown SpaceId, out-of-range, range-crossing,
        // and bad widths (including 0) all become None here.
        if !NES_SPACE_TABLE.covers(space, addr, width) {
            return None;
        }
        // The table holds WorkRam, PpuVram, and CartridgeRam;
        // covers() already pinned the id, so dispatch on (space,
        // width). A new space needs a new arm here; unknown ids cannot
        // reach this point.
        match (space, width) {
            (SPACE_WORK_RAM, 1) => self.read_byte(addr),
            (SPACE_WORK_RAM, 2) => {
                let lo = self.read_byte(addr)?;
                let hi = self.read_byte(addr + 1)?;
                Some(lo | (hi << 8))
            }
            (SPACE_WORK_RAM, 4) => {
                let mut v = 0u64;
                for i in 0..4 {
                    v |= self.read_byte(addr + i)? << (8 * i);
                }
                Some(v)
            }
            (SPACE_PPU_VRAM, 1) => self.read_vram_byte(addr),
            (SPACE_PPU_VRAM, 2) => {
                let lo = self.read_vram_byte(addr)?;
                let hi = self.read_vram_byte(addr + 1)?;
                Some(lo | (hi << 8))
            }
            (SPACE_PPU_VRAM, 4) => {
                let mut v = 0u64;
                for i in 0..4 {
                    v |= self.read_vram_byte(addr + i)? << (8 * i);
                }
                Some(v)
            }
            (SPACE_CARTRIDGE_RAM, 1) => self.read_cartridge_byte(addr),
            (SPACE_CARTRIDGE_RAM, 2) => {
                let lo = self.read_cartridge_byte(addr)?;
                let hi = self.read_cartridge_byte(addr + 1)?;
                Some(lo | (hi << 8))
            }
            (SPACE_CARTRIDGE_RAM, 4) => {
                let mut v = 0u64;
                for i in 0..4 {
                    v |= self.read_cartridge_byte(addr + i)? << (8 * i);
                }
                Some(v)
            }
            _ => None,
        }
    }

    fn registers(&self) -> &[(&'static str, u64)] {
        self.regs
            .get_or_init(|| Vec::from(self.core.cpu_registers()))
    }

    /// SPIKE-ONLY (spike/debugger-ui-prototype-3). Pattern-table images
    /// via the spike producer. Deleted with the spike branch.
    fn images(&self) -> Vec<DebugImage> {
        crate::spike_pattern::pattern_images(self.core)
    }

    fn panels(&self) -> Vec<DebugPanel> {
        Vec::new()
    }
}

#[cfg(test)]
mod tests {
    use nerust_core_traits::debugger::validate_spaces;

    use super::*;
    use crate::nrom_test_data;

    fn live_core() -> Core {
        Core::new(nrom_test_data()).expect("NROM test cartridge")
    }

    #[test]
    fn nes_spaces_validate() {
        NES_SPACE_TABLE.validate().expect("NES spaces valid");
        assert_eq!(validate_spaces(&NES_SPACES), Ok(()));
    }

    /// SPIKE-ONLY: pattern images against the live NROM core (zero CHR).
    #[test]
    fn spike_pattern_images_smoke() {
        let core = live_core();
        let dbg = NesDebugger::new(&core);
        let images = dbg.images();
        assert_eq!(images.len(), 2);
        for image in &images {
            assert_eq!((image.width, image.height), (128, 128));
            assert_eq!(image.pixels.len(), 128 * 128);
            assert_eq!(image.palette.len(), 4);
        }
        // Zero CHR decodes all-blank: shape valid, content empty.
        assert!(images.iter().all(|i| i.pixels.iter().all(|&p| p == 0)));
    }

    #[test]
    fn nes_debugger_can_be_boxed() {
        let core = live_core();
        let boxed: Box<dyn Debugger + '_> = Box::new(NesDebugger::new(&core));
        assert_eq!(boxed.spaces().len(), 3);
        assert_eq!(boxed.space_containing(0x0100), Some(SPACE_WORK_RAM));
        assert_eq!(boxed.space_containing(0x2000), Some(SPACE_PPU_VRAM));
        assert_eq!(boxed.space_containing(0x6000), Some(SPACE_CARTRIDGE_RAM));
        assert_eq!(boxed.space_containing(0x1000), Some(SPACE_WORK_RAM));
        assert_eq!(boxed.space_containing(0x4000), None);
        // Registers: 6 entries in ascending name order (kernel contract).
        let names: Vec<&str> = boxed.registers().iter().map(|(n, _)| *n).collect();
        assert_eq!(names, vec!["a", "p", "pc", "sp", "x", "y"]);
        // Live read path: fresh WRAM is readable.
        assert!(boxed.read(SPACE_WORK_RAM, 0x0100, 1).is_some());
        // Rejected inputs collapse to None.
        assert_eq!(boxed.read(SPACE_WORK_RAM, 0x1FFF, 2), None);
        assert_eq!(boxed.read(SPACE_WORK_RAM, 0x0100, 0), None);
        assert_eq!(boxed.read(SPACE_WORK_RAM, 0x0100, 3), None);
        assert_eq!(boxed.read(SpaceId(9), 0x0100, 1), None);
    }
}
