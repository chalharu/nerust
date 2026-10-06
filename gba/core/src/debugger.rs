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

use nerust_core_traits::{
    audio::StereoSample,
    debugger::{
        DebugControl, Debugger, DebuggerError, SpaceAccess, SpaceId, SpaceInfo, SpaceTable,
        StepUnit,
    },
};
use nerust_render_traits::{FrameBuffer, PixelFormat};

use crate::{console_core::GbaConsoleCore, system::GbaSystem};

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
static GBA_SPACE_TABLE: SpaceTable = SpaceTable::build(&GBA_SPACES);

/// Read-only GBA observer.
///
/// Built per use (built, read, dropped); `registers()` borrows a buffer
/// filled once in `new`.
///
/// Register names are plain `r0`-`r15` plus `cpsr`, in that order. This
/// deliberately bends the kernel's ascending-order clause at `r10`
/// (`"r10" < "r2"` lexically): conventional names won over padding.
/// Presentation must not re-sort.
///
/// `r()` ORs the user bank during the post-LDM conflict window: the
/// list reports what the register reads as, transient included —
/// deterministic at a fixed pause point all the same.
pub struct GbaDebugger<'a> {
    system: &'a GbaSystem,
    regs: Vec<(&'static str, u64)>,
}

impl<'a> GbaDebugger<'a> {
    pub fn new(system: &'a GbaSystem) -> Self {
        let regs = system.cpu.registers();
        let mut out = Vec::with_capacity(18);
        out.push(("cpsr", u64::from(regs.cpsr())));
        // Leak-free static names: the index selects a literal, so no
        // allocation and no loss of the ascending intent.
        const NAMES: [&str; 16] = [
            "r0", "r1", "r2", "r3", "r4", "r5", "r6", "r7", "r8", "r9", "r10", "r11", "r12", "r13",
            "r14", "r15",
        ];
        for (i, name) in NAMES.iter().enumerate() {
            out.push((*name, u64::from(regs.r(i))));
        }
        Self { system, regs: out }
    }
}

impl Debugger for GbaDebugger<'_> {
    fn spaces(&self) -> &[SpaceInfo] {
        &GBA_SPACES
    }

    fn space_containing(&self, addr: u32) -> Option<SpaceId> {
        GBA_SPACES
            .iter()
            .find(|s| s.range.contains(&addr))
            .map(|s| s.id)
    }

    fn read(&self, space: SpaceId, addr: u32, width: u8) -> Option<u64> {
        // Single gate: unknown SpaceId, out-of-range, range-crossing,
        // and bad widths (including 0) all become None here.
        if !GBA_SPACE_TABLE.covers(space, addr, width) {
            return None;
        }
        // Little-endian composition of physical bytes. CPU aligned-ROR
        // semantics do not apply: the debugger shows physical contents.
        let mut value = 0u64;
        for i in 0..width as u32 {
            value |= u64::from(self.system.bus.peek_byte(addr + i)?) << (8 * i);
        }
        Some(value)
    }

    fn registers(&self) -> &[(&'static str, u64)] {
        &self.regs
    }
}

/// GBA execution control.
///
/// Holds the console: frame stepping reuses the exact `render_frame`
/// path and reports the cycles it counted. The scratch buffer is
/// display-irrelevant; stepped frames are not published to any shared
/// framebuffer here. The space is read-only in phase 1, so every
/// in-range write is refused as `ReadOnlySpace`.
pub struct GbaDebugControl<'a> {
    console: &'a mut GbaConsoleCore,
    frame_slot: FrameBuffer,
    audio_sink: Vec<StereoSample>,
}

impl<'a> GbaDebugControl<'a> {
    pub fn new(console: &'a mut GbaConsoleCore) -> Self {
        let mut frame_slot = FrameBuffer::with_capacity(240, 160, PixelFormat::Rgba);
        frame_slot.resize(240, 160);
        Self {
            console,
            frame_slot,
            audio_sink: Vec::new(),
        }
    }
}

impl DebugControl for GbaDebugControl<'_> {
    fn step(&mut self, unit: StepUnit) -> Result<u64, DebuggerError> {
        match unit {
            StepUnit::Frame => self
                .console
                .render_frame_cycles(&mut self.frame_slot, &mut self.audio_sink)
                .map_err(|_| DebuggerError::Unsupported),
            StepUnit::Instruction => Err(DebuggerError::UnsupportedStepUnit(unit)),
        }
    }

    fn write_memory(
        &mut self,
        space: SpaceId,
        addr: u32,
        width: u8,
        _value: u64,
    ) -> Result<(), DebuggerError> {
        // Validation order matters: bad width first, so "width 0 with
        // unknown SpaceId" does not mask as UnknownSpace.
        if !SpaceTable::width_is_valid(width) {
            return Err(DebuggerError::BadWidth(width));
        }
        let info = GBA_SPACE_TABLE
            .get(space)
            .ok_or(DebuggerError::UnknownSpace(space))?;
        if !GBA_SPACE_TABLE.covers(space, addr, width) {
            return Err(DebuggerError::UnmappedAddress { space, addr });
        }
        if info.access == SpaceAccess::ReadOnly {
            return Err(DebuggerError::ReadOnlySpace(space));
        }
        Err(DebuggerError::ReadOnlySpace(space))
    }
}

#[cfg(test)]
mod tests {
    use std::{
        collections::HashMap,
        sync::{Arc, Mutex, atomic::AtomicBool},
    };

    use nerust_core_traits::{
        ConsoleCore as _, CoreConfig,
        debugger::{DebugControl, Debugger, DebuggerError, StepUnit, validate_spaces},
    };
    use nerust_input_traits::{EmuInput, InputStateBuffer};

    use super::*;
    use crate::input_types::GbaInputBuffer;

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

    fn test_emu_input() -> EmuInput {
        let shared: Arc<Mutex<Box<dyn InputStateBuffer>>> =
            Arc::new(Mutex::new(Box::<GbaInputBuffer>::default()));
        EmuInput::new(
            shared,
            Arc::new(AtomicBool::new(false)),
            Box::new(|| Box::<GbaInputBuffer>::default()),
        )
    }

    fn load_console() -> GbaConsoleCore {
        let mut console = GbaConsoleCore::new(test_emu_input());
        console
            .load(
                &rom(),
                &CoreConfig {
                    region: None,
                    bios_paths: HashMap::new(),
                    controllers: HashMap::new(),
                    core_options: None,
                    audio_sample_rate: None,
                },
            )
            .expect("test ROM loads");
        console
    }

    fn rom() -> Vec<u8> {
        let mut rom = vec![0; 0x4000];
        // Header, logo, and complement are filled by the finalizer.
        crate::cartridge::header::finalize_test_gba_rom(&mut rom);
        rom
    }

    fn live_system() -> GbaSystem {
        // The debugger borrows the live system; the test ROM builds one
        // the same way load does.
        GbaSystem::from_test_rom(rom()).expect("test ROM builds a system")
    }

    #[test]
    fn gba_debugger_can_be_boxed() {
        let system = live_system();
        let boxed: Box<dyn Debugger + '_> = Box::new(GbaDebugger::new(&system));
        assert_eq!(boxed.spaces().len(), 1);
        assert_eq!(boxed.space_containing(0x02000000), Some(SPACE_MEMORY));
        assert_eq!(boxed.space_containing(0x08000000), Some(SPACE_MEMORY));
        assert_eq!(boxed.space_containing(0x10000000), None);
        // Registers: cpsr then r0-r15 in order (plain names by design).
        let names: Vec<&str> = boxed.registers().iter().map(|(n, _)| *n).collect();
        let mut expected = vec!["cpsr"];
        expected.extend(
            [
                "r0", "r1", "r2", "r3", "r4", "r5", "r6", "r7", "r8", "r9", "r10", "r11", "r12",
                "r13", "r14", "r15",
            ]
            .iter(),
        );
        assert_eq!(names, expected);
        // Live read path: the ROM logo byte is observable through the
        // game-pak window (the GBA logo starts 0x24, not the GB logo).
        assert_eq!(boxed.read(SPACE_MEMORY, 0x08000004, 1), Some(0x24));
        // Fresh EWRAM is readable.
        assert!(boxed.read(SPACE_MEMORY, 0x02000000, 1).is_some());
        // Little-endian composition.
        let lo = boxed.read(SPACE_MEMORY, 0x08000004, 1).unwrap();
        let wide = boxed.read(SPACE_MEMORY, 0x08000004, 2).unwrap();
        assert_eq!(wide & 0xFF, lo);
        // Unmapped and bad inputs collapse to None.
        assert_eq!(boxed.read(SPACE_MEMORY, 0x10000000, 1), None);
        assert_eq!(boxed.read(SPACE_MEMORY, 0x0FFFFFFF, 2), None);
        assert_eq!(boxed.read(SPACE_MEMORY, 0x02000000, 0), None);
        assert_eq!(boxed.read(SPACE_MEMORY, 0x02000000, 3), None);
        assert_eq!(boxed.read(SpaceId(9), 0x02000000, 1), None);
    }

    #[test]
    fn gba_control_can_be_boxed_and_rejects_in_order() {
        let mut console = load_console();
        let mut boxed: Box<dyn DebugControl + '_> = Box::new(GbaDebugControl::new(&mut console));
        // BadWidth first: width 0 with unknown SpaceId is still BadWidth.
        assert_eq!(
            boxed.write_memory(SpaceId(9), 0x02000000, 0, 0),
            Err(DebuggerError::BadWidth(0))
        );
        assert_eq!(
            boxed.write_memory(SpaceId(9), 0x02000000, 1, 0),
            Err(DebuggerError::UnknownSpace(SpaceId(9)))
        );
        assert_eq!(
            boxed.write_memory(SPACE_MEMORY, 0x10000000, 1, 0),
            Err(DebuggerError::UnmappedAddress {
                space: SPACE_MEMORY,
                addr: 0x10000000
            })
        );
        // Phase 1 is read-only.
        assert_eq!(
            boxed.write_memory(SPACE_MEMORY, 0x02000000, 1, 0),
            Err(DebuggerError::ReadOnlySpace(SPACE_MEMORY))
        );
        // No instruction stepping yet: explicit, not silent.
        assert_eq!(
            boxed.step(StepUnit::Instruction),
            Err(DebuggerError::UnsupportedStepUnit(StepUnit::Instruction))
        );
        // Frame stepping reuses the render path and reports real cycles.
        let cycles = boxed.step(StepUnit::Frame).expect("frame step");
        assert!(cycles > 0);
        assert!(cycles <= 280_896);
    }

    #[test]
    fn gba_debugger_wiring_follows_load_state() {
        let mut empty = GbaConsoleCore::new(test_emu_input());
        assert!(empty.debugger().is_none());
        assert!(empty.debug_control().is_none());

        let mut loaded = load_console();
        assert!(loaded.debugger().is_some());
        assert!(loaded.debug_control().is_some());

        loaded.unload();
        assert!(loaded.debugger().is_none());
        assert!(loaded.debug_control().is_none());
    }
}
