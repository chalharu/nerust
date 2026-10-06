//! NES debugger: read-only observation plus controlled WRAM edits.
//!
//! Design: `debugger-trait-design.md` v7.0 §6.5. The observer is per-use
//! (built, read, dropped); `registers()` borrows a buffer filled once in
//! `new`, since `RefCell` cannot hand out borrows (E0515).

use nerust_core_traits::{
    audio::StereoSample,
    debugger::{
        DebugControl, DebugPanel, Debugger, DebuggerError, SpaceAccess, SpaceId, SpaceInfo,
        SpaceTable, StepUnit,
    },
};
use nerust_render_traits::{FrameBuffer, PixelFormat};

use crate::{Core, console_core::NesConsoleCore};

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
        &self.regs
    }

    fn panels(&self) -> Vec<DebugPanel> {
        Vec::new()
    }
}

/// NES execution control and memory editing.
///
/// Holds the console (not just the core): frame stepping reuses the
/// exact `render_frame` path and reports the cycles it counted.
/// Scratch buffers are display-irrelevant (zeroed palette); stepped
/// frames are not published to any shared framebuffer here.
pub struct NesDebugControl<'a> {
    console: &'a mut NesConsoleCore,
    frame_slot: FrameBuffer,
    audio_sink: Vec<StereoSample>,
}

impl<'a> NesDebugControl<'a> {
    pub fn new(console: &'a mut NesConsoleCore) -> Self {
        let mut frame_slot = FrameBuffer::with_capacity(
            256,
            240,
            PixelFormat::PaletteIndex {
                palette: Box::new([0u32; 256]),
            },
        );
        frame_slot.resize(256, 240);
        Self {
            console,
            frame_slot,
            audio_sink: Vec::new(),
        }
    }
}

impl DebugControl for NesDebugControl<'_> {
    fn step(&mut self, unit: StepUnit) -> Result<u64, DebuggerError> {
        match unit {
            StepUnit::Frame => self
                .console
                .render_frame_cycles(&mut self.frame_slot, &mut self.audio_sink)
                .map_err(|_| DebuggerError::Unsupported),
            // Instruction stepping is a required debug capability, but
            // needs an instruction-boundary API in the CPU core first.
            StepUnit::Instruction => Err(DebuggerError::UnsupportedStepUnit(unit)),
        }
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
        let core = self
            .console
            .core_mut()
            .map_err(|_| DebuggerError::Unsupported)?;
        for i in 0..width as u32 {
            core.poke_work_ram((addr + i) as usize, (value >> (8 * i)) as u8);
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

    #[test]
    fn nes_write_read_roundtrip_is_little_endian() {
        use nerust_input_traits::{ControllerCollection, EmuInput, InputStateBuffer};
        use std::sync::{Arc, Mutex, atomic::AtomicBool};

        use crate::{console_core::NesConsoleCore, input_types::NesInputBuffer};

        let shared: Arc<Mutex<Box<dyn InputStateBuffer>>> =
            Arc::new(Mutex::new(Box::<NesInputBuffer>::default()));
        let mut console = NesConsoleCore::new(
            nrom_test_data(),
            ControllerCollection::new(vec![]),
            EmuInput::new(
                shared,
                Arc::new(AtomicBool::new(false)),
                Box::new(|| Box::<NesInputBuffer>::default()),
            ),
        )
        .expect("console");
        {
            let mut control = NesDebugControl::new(&mut console);
            control
                .write_memory(SPACE_WORK_RAM, 0x0100, 2, 0xBEEF)
                .expect("WRAM write");
        }
        let core = console.core_ref().expect("loaded");
        let debugger = NesDebugger::new(core);
        assert_eq!(debugger.read(SPACE_WORK_RAM, 0x0100, 2), Some(0xBEEF));
        assert_eq!(debugger.read(SPACE_WORK_RAM, 0x0100, 1), Some(0xEF));
        assert_eq!(debugger.read(SPACE_WORK_RAM, 0x0101, 1), Some(0xBE));
    }
    #[test]
    fn nes_control_can_be_boxed_and_rejects_in_order() {
        use nerust_input_traits::{ControllerCollection, EmuInput, InputStateBuffer};
        use std::sync::{Arc, Mutex, atomic::AtomicBool};

        use crate::{console_core::NesConsoleCore, input_types::NesInputBuffer};

        let shared: Arc<Mutex<Box<dyn InputStateBuffer>>> =
            Arc::new(Mutex::new(Box::<NesInputBuffer>::default()));
        let mut console = NesConsoleCore::new(
            nrom_test_data(),
            ControllerCollection::new(vec![]),
            EmuInput::new(
                shared,
                Arc::new(AtomicBool::new(false)),
                Box::new(|| Box::<NesInputBuffer>::default()),
            ),
        )
        .expect("console");
        let mut boxed: Box<dyn DebugControl + '_> = Box::new(NesDebugControl::new(&mut console));
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
        // Frame stepping reuses the render path and reports real cycles.
        let cycles = boxed.step(StepUnit::Frame).expect("frame step");
        assert!(cycles > 0);
    }
}
