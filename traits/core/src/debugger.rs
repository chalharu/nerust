//! Kernel debugger types: read-only observation plus controlled execution.
//! Value types and two traits only; observation is `&self`, control is a
//! separate `&mut` trait.

use std::ops::RangeInclusive;
use std::sync::Arc;

// ---------------------------------------------------------------------------
// Spaces
// ---------------------------------------------------------------------------

/// Address-space identifier. Index into [`Debugger::spaces()`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SpaceId(pub u16);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpaceAccess {
    ReadOnly,
    ReadWrite,
}

/// Static description of one address space.
///
/// `key` is a stable identifier for logs and error display, and for
/// resolving ids from snapshots at configuration time (open, not per
/// frame). It is never used for per-frame branching — branch on
/// [`SpaceId`] instead.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpaceInfo {
    pub id: SpaceId,
    pub key: &'static str,
    pub name: &'static str,
    /// Address bit width, used by presentation for digit alignment.
    pub address_bits: u8,
    /// Covered address range, inclusive on both ends. Must be
    /// non-empty and disjoint from every other space in the table.
    pub range: RangeInclusive<u32>,
    pub access: SpaceAccess,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpaceError {
    Overlap {
        a: SpaceId,
        b: SpaceId,
    },
    OutOfOrder {
        space: SpaceId,
    },
    /// `start > end`: `contains` is always false, a ghost space.
    EmptyRange {
        space: SpaceId,
    },
    /// `SpaceId` value does not match the table index.
    IdIndexMismatch {
        id: SpaceId,
        index: usize,
    },
    /// A `read_only` window names an unknown space, is empty, or
    /// escapes its space's range.
    ReadOnlyOutOfRange {
        space: SpaceId,
    },
}

/// Verify that a space list is sorted, disjoint, and id/index consistent.
///
/// The id/index check is folded in on purpose: a separate test could be
/// forgotten, and then a reordered table would silently mis-resolve
/// `space_containing` while still passing validation.
pub fn validate_spaces(spaces: &[SpaceInfo]) -> Result<(), SpaceError> {
    for w in spaces.windows(2) {
        if w[0].range.start() > w[1].range.start() {
            return Err(SpaceError::OutOfOrder { space: w[1].id });
        }
        if w[0].range.end() >= w[1].range.start() {
            return Err(SpaceError::Overlap {
                a: w[0].id,
                b: w[1].id,
            });
        }
    }
    for (i, s) in spaces.iter().enumerate() {
        if s.id.0 as usize != i {
            return Err(SpaceError::IdIndexMismatch { id: s.id, index: i });
        }
        if s.range.start() > s.range.end() {
            return Err(SpaceError::EmptyRange { space: s.id });
        }
    }
    Ok(())
}

/// Static address-space table: fixed per machine, `&'static`, no ownership.
/// Refusal is table data, not backend branches; check exact writability
/// with [`SpaceTable::check_write`] instead of trusting `access` alone.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SpaceTable {
    pub entries: &'static [SpaceInfo],
    pub read_only: &'static [(SpaceId, u32, u32)],
}

impl SpaceTable {
    pub const fn build(entries: &'static [SpaceInfo]) -> Self {
        Self {
            entries,
            read_only: &[],
        }
    }

    pub const fn build_with_read_only(
        entries: &'static [SpaceInfo],
        read_only: &'static [(SpaceId, u32, u32)],
    ) -> Self {
        Self { entries, read_only }
    }

    /// Return the space for `id`, checking id/index consistency.
    /// Mismatches yield `None` instead of a wrong space.
    pub fn get(&self, id: SpaceId) -> Option<&SpaceInfo> {
        self.entries.get(id.0 as usize).filter(|s| s.id == id)
    }

    /// Sortedness / disjointness check. Call from each core's unit test.
    pub fn validate(&self) -> Result<(), SpaceError> {
        validate_spaces(self.entries)?;
        for (space, start, end) in self.read_only {
            let info = self
                .get(*space)
                .ok_or(SpaceError::ReadOnlyOutOfRange { space: *space })?;
            if start > end || *start < *info.range.start() || *end > *info.range.end() {
                return Err(SpaceError::ReadOnlyOutOfRange { space: *space });
            }
        }
        Ok(())
    }

    /// `width` is 1, 2, or 4. Zero is invalid.
    pub fn width_is_valid(width: u8) -> bool {
        matches!(width, 1 | 2 | 4)
    }

    /// Whether `width` bytes from `addr` in space `id` are fully covered.
    ///
    /// The single home of the range arithmetic: `read` and `write_memory`
    /// both use it so they cannot diverge. Invalid widths (including 0)
    /// and unknown `SpaceId`s yield `false`.
    pub fn covers(&self, id: SpaceId, addr: u32, width: u8) -> bool {
        match (self.get(id), width) {
            (Some(s), 1 | 2 | 4) => {
                // checked_add: addr near u32::MAX must be false, not panic.
                match addr.checked_add(u32::from(width) - 1) {
                    Some(end) => addr >= *s.range.start() && end <= *s.range.end(),
                    None => false,
                }
            }
            _ => false,
        }
    }

    /// Write-side validation in load-bearing order. The single home of
    /// the refusal rules; every `DebugControl::write_memory` starts here.
    pub fn check_write(&self, space: SpaceId, addr: u32, width: u8) -> Result<(), DebuggerError> {
        if !Self::width_is_valid(width) {
            return Err(DebuggerError::BadWidth(width));
        }
        let info = self.get(space).ok_or(DebuggerError::UnknownSpace(space))?;
        if !self.covers(space, addr, width) {
            return Err(DebuggerError::UnmappedAddress { space, addr });
        }
        if info.access == SpaceAccess::ReadOnly {
            return Err(DebuggerError::ReadOnlySpace(space));
        }
        // `covers` pinned `addr + width - 1` without overflow, so the
        // recomputation cannot wrap.
        let end = addr + u32::from(width) - 1;
        if self
            .read_only
            .iter()
            .any(|(s, start, stop)| *s == space && addr <= *stop && end >= *start)
        {
            return Err(DebuggerError::ReadOnlySpace(space));
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Debugger — read-only observation
// ---------------------------------------------------------------------------

/// Read-only core observer. `&self` is a type-level non-invasion guarantee:
/// observation must not advance prefetchers, charge wait cycles, or touch
/// bus history.
pub trait Debugger {
    /// All spaces, satisfying `validate_spaces`. Borrowed from the core's
    /// static table, so no allocation happens here.
    fn spaces(&self) -> &[SpaceInfo];

    /// Space covering `addr`, or `None` when unmapped.
    fn space_containing(&self, addr: u32) -> Option<SpaceId>;

    /// Read `width` bytes (1 / 2 / 4) at `addr` in `space`.
    ///
    /// Hardware-faithful: mapped reads match executed reads, unmapped
    /// yields the stable latch, no side effects. Out-of-range, straddling,
    /// and unknown spaces/widths yield `None`; range checking is the
    /// reader's duty. `Option`, not `Result`: invalid width is not
    /// distinguished from unmapped on this hot path.
    fn read(&self, space: SpaceId, addr: u32, width: u8) -> Option<u64>;

    /// Contiguous byte read for hex dumps. Stops at the range end.
    fn read_bytes(&self, space: SpaceId, start: u32, out: &mut [u8]) -> usize {
        let mut n = 0;
        for (i, slot) in out.iter_mut().enumerate() {
            match self.read(space, start + i as u32, 1) {
                Some(v) => *slot = v as u8,
                None => break,
            }
            n += 1;
        }
        n
    }

    /// Name → value register list, borrowed, zero allocation.
    /// Backend-defined stable order; anchor via
    /// [`Debugger::program_counter`], never by name-scanning.
    fn registers(&self) -> &[(&'static str, u64)];

    /// System-specific observation tables. Not called per frame.
    fn panels(&self) -> Vec<DebugPanel> {
        Vec::new()
    }

    /// Program counter for disassembly anchoring, if the system names
    /// one. Defaults to `None`: presentation shows an explicit
    /// unanchored degenerate instead of guessing. Register names stay
    /// architectural (`r15`, never a `pc` alias); anchoring does not
    /// go through the register list.
    fn program_counter(&self) -> Option<u32> {
        None
    }

    /// System images (pattern tables). Empty by default; cores with
    /// image views override it. Cold path: allocation is expected,
    /// and callers never invoke it per frame.
    fn images(&self) -> Vec<DebugImage> {
        Vec::new()
    }

    /// Disassemble `count` rows from `addr`. Empty by default; cores
    /// with a disassembler override it. Short reads at the range end
    /// decode as raw bytes: a truncated tail never panics and never
    /// fabricates instructions. Cold path like `images`.
    fn disassemble(&self, _addr: u32, _count: u16) -> Vec<DisasmLine> {
        Vec::new()
    }
}

// ---------------------------------------------------------------------------
// Disassembly — one decoded instruction row
// ---------------------------------------------------------------------------

/// A single disassembled instruction row. `bytes` holds up to 4
/// instruction bytes (ARM needs 4, 6502 needs 3, Thumb needs 2),
/// `len` the used prefix.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct DisasmLine {
    /// Instruction address.
    pub addr: u32,
    /// Raw instruction bytes (first `len` entries valid).
    pub bytes: [u8; 4],
    /// Instruction length in bytes (1-4).
    pub len: u8,
    /// Human-readable text, e.g. `JMP $C5F5`.
    pub text: String,
    /// True when this row holds the current PC.
    pub is_pc: bool,
    /// Follow address for plain address operands, e.g. relative
    /// branches and absolute jumps. Presentation navigates from this
    /// and never scrapes `text`. `None` for immediates, indexed and
    /// indirect forms, implied instructions, and raw bytes.
    pub target: Option<u32>,
}

// ---------------------------------------------------------------------------
// Panels — system-specific observation tables
// ---------------------------------------------------------------------------

/// Machine-readable observation table (e.g. disassembly, PPU state).
/// Values are plain data only: no domain types cross into the kernel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DebugPanel {
    pub id: &'static str,
    pub label_id: &'static str,
    pub columns: &'static [PanelColumn],
    pub rows: Vec<PanelRow>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PanelColumn {
    pub id: &'static str,
    pub label_id: &'static str,
    pub kind: ColumnKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColumnKind {
    Text,
    Hex { digits: u8 },
    Dec,
    Bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PanelRow {
    pub key: &'static str,
    pub cells: Vec<CellValue>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CellValue {
    Text(String),
    U64(u64),
    Bool(bool),
}

/// One system image. `pixels` are row-major indices into `palette`;
/// the frontend blits mechanically with no system knowledge.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DebugImage {
    pub id: &'static str,
    pub label_id: &'static str,
    pub width: u32,
    pub height: u32,
    pub format: ImageFormat,
    pub palette: Vec<[u8; 3]>,
    pub pixels: Vec<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageFormat {
    /// 2 bits per pixel, values 0-3 into a 4-entry palette.
    Indexed2bpp,
    /// 4 bits per pixel, values 0-15 into a 16-entry palette.
    Indexed4bpp,
}

/// Expand a 15-bit BGR color (Game Boy / GBA native) to RGB888.
/// Shared tile-imaging helper; cores with paletted images use it so
/// every viewer shows identical colors.
pub fn rgb555_to_rgb888(value: u16) -> [u8; 3] {
    // Bits 0-4 red, 5-9 green, 10-14 blue on both systems.
    let expand = |v: u16| ((u32::from(v) * 255 + 15) / 31) as u8;
    [
        expand(value & 31),
        expand((value >> 5) & 31),
        expand((value >> 10) & 31),
    ]
}

// ---------------------------------------------------------------------------
// Snapshots — published observation results (pure data)
// ---------------------------------------------------------------------------

/// Max bytes per inspect response (256 rows at 16 bytes/row).
pub const MAX_DUMP_BYTES: usize = 4096;
pub const DEFAULT_DUMP_ROWS: u16 = 32;
pub const DUMP_ROW_BYTES: usize = 16;

/// One debugger drain's page sizes. The drain fetches a scroll
/// window (not just the visible rows) so both twins get native
/// wheel/viewport scrolling: the viewport shows a slice, Prev/Next
/// move by the whole window. Single-byte reads use `rows: 1`.
pub const DEBUG_DUMP_PAGE_ROWS: u16 = 32;
pub const DEBUG_DISASM_ROWS: u16 = 32;

/// Per-frame published observation. Holds results only — staleness and
/// last-error tracking live in the application layer.
pub struct DebuggerSnapshot {
    /// `false` when no ROM is loaded or the core has no debugger.
    /// Distinguishes "unavailable" from "empty registers".
    pub available: bool,
    /// Static table reference. No copy, no allocation.
    pub spaces: &'static SpaceTable,
    /// Dynamic per-frame values.
    pub registers: Arc<[(&'static str, u64)]>,
    pub execution: ExecutionInfo,
}

/// One hex-dump row. `valid` counts readable bytes so a short final row
/// is distinguishable from genuinely-zero memory.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct HexRow {
    pub addr: u32,
    pub valid: u8,
    pub bytes: [u8; DUMP_ROW_BYTES],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExecutionInfo {
    pub paused: bool,
    pub frame: u64,
}

/// Successful inspect response content.
#[derive(Debug, Clone)]
pub struct InspectResult {
    pub dump: MemoryDump,
    pub panels: Arc<[DebugPanel]>,
    /// Register snapshot at capture, same paused moment as the dump.
    /// Names follow `Debugger::registers` (ascending order).
    pub registers: Arc<[(&'static str, u64)]>,
    /// Frame number at capture, for UI staleness judgment.
    pub captured_at_frame: u64,
}

#[derive(Debug, Clone)]
pub struct MemoryDump {
    pub space: SpaceId,
    pub base: u32,
    pub rows: Arc<[HexRow]>,
}

// ---------------------------------------------------------------------------
// Control — execution stepping and memory writes
// ---------------------------------------------------------------------------

/// Execution granularity. Shared by `emu-thread` and `gui/shell`,
/// hence owned by the kernel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StepUnit {
    Instruction,
    Frame,
}

/// Inspect request. Lives in the kernel because `EmuCommand` uses it;
/// placing it in `gui/shell` would cycle the dependency.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InspectRequest {
    pub space: Option<SpaceId>,
    pub addr: Option<u32>,
    pub rows: u16,
}

/// Memory-write request. Used by `EmuCommand::WriteMemory`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MemoryWrite {
    pub space: SpaceId,
    pub addr: u32,
    pub width: u8,
    pub value: u64,
}

/// Batched debugger snapshot: inspect + anchor + disassembly in one
/// thread crossing instead of three sequential round trips (each
/// with its own timeout on the UI thread). Per-part contracts match
/// the individual commands: paused-gated inspect, ungated PC and
/// decode. Failure semantics are preserved exactly: an inspect
/// failure skips the rest (as today), a PC failure yields no anchor
/// and no disassembly, disassembly failure yields no rows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SnapshotRequest {
    pub inspect: InspectRequest,
    /// Pinned anchor; `None` is follow-PC mode (anchor resolves via
    /// the program counter inside the batch).
    pub anchor: Option<u32>,
    pub disasm_rows: u16,
}

/// Batched snapshot result. `disasm` is empty on skip/failure,
/// mirroring the individual-call fallbacks (never an error).
#[derive(Debug, Clone)]
pub struct SnapshotResult {
    pub inspect: Result<InspectResult, InspectError>,
    pub anchor: Option<u32>,
    pub disasm: Vec<DisasmLine>,
}

/// Debug-capability failure reasons. Data-only; display resolves
/// outside the kernel. `Unsupported` covers unimplemented and unknown
/// register names alike; add a variant instead of overloading if the
/// two must ever be told apart.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DebuggerError {
    /// The core supports neither debugger nor debug control.
    Unsupported,
    /// This core does not implement the requested [`StepUnit`]
    /// (e.g. no single-instruction stepping).
    UnsupportedStepUnit(StepUnit),
    /// `SpaceId` not present in `spaces()`. No key exists by definition.
    UnknownSpace(SpaceId),
    /// Space resolved, but the address is unmapped within it.
    UnmappedAddress { space: SpaceId, addr: u32 },
    /// Write refused: [`SpaceAccess::ReadOnly`].
    ReadOnlySpace(SpaceId),
    /// Width other than 1 / 2 / 4. Write-side only; reads use `None`.
    BadWidth(u8),
}

/// `EmuCommand::DebuggerInspect` failure reasons.
///
/// Defined in the kernel next to `EmuCommand` (not in `traits/emu-thread`
/// as §5.2 sketched: the command type lives here, so its reply type must
/// too — otherwise the crates cycle). The layering still holds: `NotPaused`
/// is produced only by the loop guard (infrastructure), every other
/// variant arrives wrapped from domain code.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InspectError {
    /// Not paused, so inspection is refused. Produced by the loop guard;
    /// domain code can never observe `paused` and never produces this.
    NotPaused,
    /// The core refused the debug capability. Reason comes from domain.
    Core(DebuggerError),
}

/// Execution control and memory editing. Separate from [`Debugger`] so
/// that `&self` observation never implies write permission, and so that
/// cores with observation but no single-stepping stay expressible.
///
/// `Send` is intentionally not required: control objects are created
/// and consumed inside emu-thread command handling and never cross
/// threads.
pub trait DebugControl {
    /// Advance execution by one unit. Returns cycles advanced.
    fn step(&mut self, unit: StepUnit) -> Result<u64, DebuggerError>;

    /// Edit memory (memory-viewer path).
    ///
    /// **`Result<(), DebuggerError>` であり `bool` ではない。**
    /// `UnknownSpace` / `ReadOnlySpace` / `BadWidth` を返すために理由が
    /// 必要で、`bool` ではこれらの variant が producers を持たない。
    fn write_memory(
        &mut self,
        space: SpaceId,
        addr: u32,
        width: u8,
        value: u64,
    ) -> Result<(), DebuggerError>;

    /// Edit one named register (register-viewer path). `name` is one
    /// of [`Debugger::registers`] names; backends mask `value` to the
    /// register width. Unknown names report `Unsupported` (no new
    /// error variant: the name travels as data, and the refusal
    /// reason stays one word). Defaults to `Unsupported` so
    /// observation-only controls stay expressible.
    fn write_register(&mut self, _name: &str, _value: u64) -> Result<(), DebuggerError> {
        Err(DebuggerError::Unsupported)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debugger_is_dyn_compatible() {
        // Compiles only if every Debugger method is dyn-compatible:
        // no RPITIT, no `impl Trait`. No runtime effect by construction.
        fn _assert_dyn<T: Debugger + ?Sized>() {}
        let _f: fn(&dyn Debugger) = |_| {};
    }

    #[test]
    fn debug_control_is_dyn_compatible() {
        fn _assert_dyn<T: DebugControl + ?Sized>() {}
        let _f: fn(&mut dyn DebugControl) = |_| {};
    }

    struct EmptyDebugger;

    impl Debugger for EmptyDebugger {
        fn spaces(&self) -> &[SpaceInfo] {
            &[]
        }

        fn space_containing(&self, _addr: u32) -> Option<SpaceId> {
            None
        }

        fn read(&self, _space: SpaceId, _addr: u32, _width: u8) -> Option<u64> {
            None
        }

        fn registers(&self) -> &[(&'static str, u64)] {
            &[]
        }
    }

    #[test]
    fn debugger_defaults_are_explicitly_empty() {
        let debugger = EmptyDebugger;
        assert_eq!(debugger.program_counter(), None);
        assert!(debugger.images().is_empty());
        assert!(debugger.disassemble(0xC000, 8).is_empty());
        assert!(debugger.panels().is_empty());
    }

    #[test]
    fn space_table_covers_rejects_bad_width_and_unknown_space() {
        static ENTRIES: [SpaceInfo; 1] = [SpaceInfo {
            id: SpaceId(0),
            key: "wram",
            name: "WRAM",
            address_bits: 13,
            range: 0x0000..=0x1FFF,
            access: SpaceAccess::ReadWrite,
        }];
        let table = SpaceTable::build(&ENTRIES);
        assert!(table.validate().is_ok());
        assert!(table.covers(SpaceId(0), 0x0100, 1));
        assert!(table.covers(SpaceId(0), 0x1FFF, 1));
        assert!(!table.covers(SpaceId(0), 0x1FFF, 2));
        assert!(!table.covers(SpaceId(0), 0x0100, 0));
        assert!(!table.covers(SpaceId(0), 0x0100, 3));
        assert!(!table.covers(SpaceId(9), 0x0100, 1));
        assert!(!SpaceTable::width_is_valid(0));
        // Overflow edge: addr near u32::MAX is false, never panic.
        assert!(!table.covers(SpaceId(0), 0xFFFF_FFFF, 4));
        assert!(!table.covers(SpaceId(0), 0xFFFF_FFFF, 1));
    }

    #[test]
    fn check_write_orders_refusals_and_honors_read_only_windows() {
        static ENTRIES: [SpaceInfo; 2] = [
            SpaceInfo {
                id: SpaceId(0),
                key: "wram",
                name: "WRAM",
                address_bits: 13,
                range: 0x0000..=0x1FFF,
                access: SpaceAccess::ReadWrite,
            },
            SpaceInfo {
                id: SpaceId(1),
                key: "rom",
                name: "ROM",
                address_bits: 13,
                range: 0x2000..=0x3FFF,
                access: SpaceAccess::ReadOnly,
            },
        ];
        static READ_ONLY: [(SpaceId, u32, u32); 1] = [(SpaceId(0), 0x0000, 0x00FF)];
        let table = SpaceTable::build_with_read_only(&ENTRIES, &READ_ONLY);
        assert!(table.validate().is_ok());
        // Load-bearing order: bad width first, so it never masks as
        // an unknown space.
        assert_eq!(
            table.check_write(SpaceId(9), 0x0100, 0),
            Err(DebuggerError::BadWidth(0))
        );
        assert_eq!(
            table.check_write(SpaceId(9), 0x0100, 1),
            Err(DebuggerError::UnknownSpace(SpaceId(9)))
        );
        assert_eq!(
            table.check_write(SpaceId(0), 0x2000, 1),
            Err(DebuggerError::UnmappedAddress {
                space: SpaceId(0),
                addr: 0x2000
            })
        );
        // Whole-space refusal, then window refusal.
        assert_eq!(
            table.check_write(SpaceId(1), 0x2000, 1),
            Err(DebuggerError::ReadOnlySpace(SpaceId(1)))
        );
        assert_eq!(
            table.check_write(SpaceId(0), 0x0010, 1),
            Err(DebuggerError::ReadOnlySpace(SpaceId(0)))
        );
        // A write straddling a window refuses instead of half-landing.
        assert_eq!(
            table.check_write(SpaceId(0), 0x00FE, 4),
            Err(DebuggerError::ReadOnlySpace(SpaceId(0)))
        );
        assert_eq!(table.check_write(SpaceId(0), 0x0100, 4), Ok(()));
    }

    #[test]
    fn validate_spaces_rejects_overlap_disorder_gaps_and_ghosts() {
        let ok = [
            SpaceInfo {
                id: SpaceId(0),
                key: "a",
                name: "A",
                address_bits: 8,
                range: 0x00..=0x0F,
                access: SpaceAccess::ReadOnly,
            },
            SpaceInfo {
                id: SpaceId(1),
                key: "b",
                name: "B",
                address_bits: 8,
                range: 0x10..=0x1F,
                access: SpaceAccess::ReadOnly,
            },
        ];
        assert!(validate_spaces(&ok).is_ok());

        let mut overlap = ok.clone();
        overlap[1].range = 0x0F..=0x1F;
        assert_eq!(
            validate_spaces(&overlap),
            Err(SpaceError::Overlap {
                a: SpaceId(0),
                b: SpaceId(1)
            })
        );

        let mut disorder = ok.clone();
        disorder.swap(0, 1);
        assert_eq!(
            validate_spaces(&disorder),
            Err(SpaceError::OutOfOrder { space: SpaceId(0) })
        );

        let mut ghost = ok.clone();
        // Built from variables so the empty range is a runtime value,
        // not a literal the compiler rejects outright.
        let (ghost_start, ghost_end) = (0x20u32, 0x1Fu32);
        ghost[1].range = ghost_start..=ghost_end;
        assert_eq!(
            validate_spaces(&ghost),
            Err(SpaceError::EmptyRange { space: SpaceId(1) })
        );

        let mut mismatch = ok.clone();
        mismatch[1].id = SpaceId(7);
        assert_eq!(
            validate_spaces(&mismatch),
            Err(SpaceError::IdIndexMismatch {
                id: SpaceId(7),
                index: 1
            })
        );
    }

    #[test]
    fn validate_rejects_escaping_read_only_windows() {
        static ENTRIES: [SpaceInfo; 1] = [SpaceInfo {
            id: SpaceId(0),
            key: "wram",
            name: "WRAM",
            address_bits: 13,
            range: 0x0000..=0x1FFF,
            access: SpaceAccess::ReadWrite,
        }];
        static ESCAPING: [(SpaceId, u32, u32); 1] = [(SpaceId(0), 0x1000, 0x2000)];
        static GHOST: [(SpaceId, u32, u32); 1] = [(SpaceId(9), 0x0010, 0x001F)];
        assert_eq!(
            SpaceTable::build_with_read_only(&ENTRIES, &ESCAPING).validate(),
            Err(SpaceError::ReadOnlyOutOfRange { space: SpaceId(0) })
        );
        assert_eq!(
            SpaceTable::build_with_read_only(&ENTRIES, &GHOST).validate(),
            Err(SpaceError::ReadOnlyOutOfRange { space: SpaceId(9) })
        );
    }
}
