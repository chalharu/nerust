//! NES debugger: read-only observation plus controlled WRAM edits.
//!
//! Design: `debugger-trait-design.md` v7.0 §6.5. The observer is per-use
//! (built, read, dropped); `registers()` borrows a buffer filled once in
//! `new`, since `RefCell` cannot hand out borrows (E0515).

use nerust_core_traits::debugger::{
    DebugControl, DebugPanel, Debugger, DebuggerError, SpaceAccess, SpaceId, SpaceInfo, SpaceTable,
    StepUnit,
};

use crate::Core;

/// Work RAM with CPU mirrors ($0000-$07FF mirrored to $1FFF).
/// Matches `peek_work_ram` semantics exactly.
pub const SPACE_WORK_RAM: SpaceId = SpaceId(0);

static NES_SPACES: [SpaceInfo; 1] = [SpaceInfo {
    id: SPACE_WORK_RAM,
    key: "wram",
    name: "WRAM",
    address_bits: 13,
    range: 0x0000..=0x1FFF,
    access: SpaceAccess::ReadWrite,
}];

static NES_SPACE_TABLE: SpaceTable = SpaceTable::build(&NES_SPACES);

/// Read-only NES observer.
pub struct NesDebugger<'a> {
    core: &'a Core,
    /// `registers()` backing buffer. Filled once in `new`: the observer
    /// is a snapshot, and must not go stale mid-frame, so callers
    /// rebuild it per use instead of holding it across frames.
    regs: Vec<(&'static str, u64)>,
}

impl<'a> NesDebugger<'a> {
    pub fn new(core: &'a Core) -> Self {
        Self {
            core,
            regs: Vec::from(core.cpu_registers()),
        }
    }

    fn read_byte(&self, addr: u32) -> Option<u64> {
        self.core.peek_work_ram(addr as usize).map(u64::from)
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
        // The table holds exactly one space and covers() already pinned
        // the id, so dispatch on width only. A new space needs a new
        // arm here; the assert fails fast in tests if forgotten.
        debug_assert_eq!(space, SPACE_WORK_RAM);
        match width {
            1 => self.read_byte(addr),
            2 => {
                let lo = self.read_byte(addr)?;
                let hi = self.read_byte(addr + 1)?;
                Some(lo | (hi << 8))
            }
            4 => {
                let mut v = 0u64;
                for i in 0..4 {
                    v |= self.read_byte(addr + i)? << (8 * i);
                }
                Some(v)
            }
            _ => None,
        }
    }

    fn registers(&self) -> &[(&'static str, u64)] {
        &self.regs
    }

    fn panels(&self) -> Vec<DebugPanel> {
        Vec::new()
    }
}

/// NES execution control and memory editing.
pub struct NesDebugControl<'a> {
    core: &'a mut Core,
}

impl<'a> NesDebugControl<'a> {
    pub fn new(core: &'a mut Core) -> Self {
        Self { core }
    }
}

impl DebugControl for NesDebugControl<'_> {
    fn step(&mut self, unit: StepUnit) -> Result<u64, DebuggerError> {
        // No cycle-accounted entry point exists yet: single-stepping has
        // no instruction-boundary API, and frame stepping has no
        // reported cycle count. Both stay explicit until wired.
        Err(DebuggerError::UnsupportedStepUnit(unit))
    }

    fn write_memory(
        &mut self,
        space: SpaceId,
        addr: u32,
        width: u8,
        value: u64,
    ) -> Result<(), DebuggerError> {
        // Validation order matters: bad width first, so "width 0 with
        // unknown SpaceId" does not mask as UnknownSpace.
        if !SpaceTable::width_is_valid(width) {
            return Err(DebuggerError::BadWidth(width));
        }
        let info = NES_SPACE_TABLE
            .get(space)
            .ok_or(DebuggerError::UnknownSpace(space))?;
        if !NES_SPACE_TABLE.covers(space, addr, width) {
            return Err(DebuggerError::UnmappedAddress { space, addr });
        }
        if info.access == SpaceAccess::ReadOnly {
            return Err(DebuggerError::ReadOnlySpace(space));
        }
        for i in 0..width as u32 {
            self.core
                .poke_work_ram((addr + i) as usize, (value >> (8 * i)) as u8);
        }
        Ok(())
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

    #[test]
    fn nes_debugger_can_be_boxed() {
        let core = live_core();
        let boxed: Box<dyn Debugger + '_> = Box::new(NesDebugger::new(&core));
        assert_eq!(boxed.spaces().len(), 1);
        assert_eq!(boxed.space_containing(0x0100), Some(SPACE_WORK_RAM));
        assert_eq!(boxed.space_containing(0x2000), None);
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

    #[test]
    fn nes_write_read_roundtrip_is_little_endian() {
        let mut core = live_core();
        let mut control = NesDebugControl::new(&mut core);
        control
            .write_memory(SPACE_WORK_RAM, 0x0100, 2, 0xBEEF)
            .expect("WRAM write");
        let debugger = NesDebugger::new(&*control.core);
        assert_eq!(debugger.read(SPACE_WORK_RAM, 0x0100, 2), Some(0xBEEF));
        assert_eq!(debugger.read(SPACE_WORK_RAM, 0x0100, 1), Some(0xEF));
        assert_eq!(debugger.read(SPACE_WORK_RAM, 0x0101, 1), Some(0xBE));
    }

    #[test]
    fn nes_control_can_be_boxed_and_rejects_in_order() {
        let mut core = live_core();
        let mut boxed: Box<dyn DebugControl + '_> = Box::new(NesDebugControl::new(&mut core));
        // BadWidth first: width 0 with unknown SpaceId is still BadWidth.
        assert_eq!(
            boxed.write_memory(SpaceId(9), 0x0100, 0, 0),
            Err(DebuggerError::BadWidth(0))
        );
        assert_eq!(
            boxed.write_memory(SpaceId(9), 0x0100, 1, 0),
            Err(DebuggerError::UnknownSpace(SpaceId(9)))
        );
        assert_eq!(
            boxed.write_memory(SPACE_WORK_RAM, 0x2000, 1, 0),
            Err(DebuggerError::UnmappedAddress {
                space: SPACE_WORK_RAM,
                addr: 0x2000
            })
        );
        assert_eq!(
            boxed.write_memory(SPACE_WORK_RAM, 0x1FFF, 2, 0),
            Err(DebuggerError::UnmappedAddress {
                space: SPACE_WORK_RAM,
                addr: 0x1FFF
            })
        );
        // Step has no cycle-accounted entry point yet: explicit, not silent.
        assert_eq!(
            boxed.step(StepUnit::Instruction),
            Err(DebuggerError::UnsupportedStepUnit(StepUnit::Instruction))
        );
        assert_eq!(
            boxed.step(StepUnit::Frame),
            Err(DebuggerError::UnsupportedStepUnit(StepUnit::Frame))
        );
    }
}
