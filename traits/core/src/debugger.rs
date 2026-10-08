//! Kernel debugger types — read-only observation plus controlled execution.
//!
//! Design: `debugger-trait-design.md` v7.0. This module carries only
//! value types and two traits; it owns no threads, no UI state, and no
//! domain types beyond `&'static str` / `u64` / `bool` / `String`.
//!
//! The central asymmetry is intentional: observation (`Debugger`) is
//! `&self` and non-invasive, while control (`DebugControl`) is a
//! separate trait reached through `ConsoleCore` with `&mut self`.

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

/// Static address-space table. The address map is fixed per machine and
/// never changes across load/unload, so the table is a `&'static` slice:
/// no allocation, no ownership, no lifetime management.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SpaceTable {
    pub entries: &'static [SpaceInfo],
}

impl SpaceTable {
    pub const fn build(entries: &'static [SpaceInfo]) -> Self {
        Self { entries }
    }

    /// Return the space for `id`, checking id/index consistency.
    /// Mismatches yield `None` instead of a wrong space.
    pub fn get(&self, id: SpaceId) -> Option<&SpaceInfo> {
        self.entries.get(id.0 as usize).filter(|s| s.id == id)
    }

    /// Sortedness / disjointness check. Call from each core's unit test.
    pub fn validate(&self) -> Result<(), SpaceError> {
        validate_spaces(self.entries)
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
    /// Contract, from measured hardware behavior:
    ///
    /// - Mapped regions return the same bytes an executed read would.
    /// - Unmapped regions return the stable open-bus latch value.
    /// - No wait-cycle charging, no bus-history updates.
    /// - `addr` outside `space`'s range yields `None`: cross-space
    ///   reads are never silently allowed. Range checking is the
    ///   reader's duty — `space_containing` returning `None` alone
    ///   does not guarantee it.
    /// - Widths other than 1 / 2 / 4 (including 0), reads crossing the
    ///   range end, and unknown `SpaceId`s yield `None`.
    ///
    /// `read` returns `Option`, not `Result`: it sits on the ROM-test
    /// hot path, and an invalid width is intentionally not distinguished
    /// from unmapped. `BadWidth` is a write-side-only error.
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
    ///
    /// Contract: names in ascending order, so presentation never sorts.
    fn registers(&self) -> &[(&'static str, u64)];

    /// System-specific observation tables. Not called per frame.
    fn panels(&self) -> Vec<DebugPanel> {
        Vec::new()
    }

    /// SPIKE-ONLY (spike/debugger-ui-prototype-3). System images
    /// (pattern tables, nametables). Empty by default; cores with
    /// image views override it. Deleted with the spike branch: the
    /// production shape is decided from spike findings.
    fn images(&self) -> Vec<DebugImage> {
        Vec::new()
    }
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

/// SPIKE-ONLY (spike/debugger-ui-prototype-3). One system image.
/// `pixels` are row-major indices into `palette` (2 bits used when
/// `format` is indexed-2bpp); the frontend blits mechanically.
/// Deleted with the spike branch.
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
}

// ---------------------------------------------------------------------------
// Snapshots — published observation results (pure data)
// ---------------------------------------------------------------------------

/// Max bytes per inspect response (256 rows at 16 bytes/row).
pub const MAX_DUMP_BYTES: usize = 4096;
pub const DEFAULT_DUMP_ROWS: u16 = 32;
pub const DUMP_ROW_BYTES: usize = 16;

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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
pub struct InspectResult {
    pub dump: MemoryDump,
    pub panels: Arc<[DebugPanel]>,
    /// Register snapshot at capture, same paused moment as the dump.
    /// Names follow `Debugger::registers` (ascending order).
    pub registers: Arc<[(&'static str, u64)]>,
    /// Frame number at capture, for UI staleness judgment.
    pub captured_at_frame: u64,
}

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

/// Debug-capability failure reasons.
///
/// Errors carry their own data only; display keys resolve through
/// [`DebuggerError::space_key`], so `UnknownSpace` never fabricates one.
/// All variants are produced by domain (core) code alone.
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

impl DebuggerError {
    /// Resolve the display key. `None` for variants without a table entry.
    pub fn space_key(&self, table: &SpaceTable) -> Option<&'static str> {
        let id = match self {
            Self::UnknownSpace(id) | Self::ReadOnlySpace(id) => *id,
            Self::UnmappedAddress { space, .. } => *space,
            _ => return None,
        };
        table.get(id).map(|s| s.key)
    }
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
}
