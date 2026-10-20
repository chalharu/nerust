//! Shared debugger snapshot assembly: one place turning a thread batch into sections.
//! Both the blocking snapshot and the async drain flight build from these parts.

use nerust_core_traits::debugger::{DisasmLine, HexRow, SnapshotResult};

/// Structured sections of one snapshot batch; unavailable sections stay empty upstream.
#[derive(Default)]
pub(crate) struct SnapshotParts {
    /// Register `(name, value)` pairs in thread order.
    pub regs: Vec<(&'static str, u64)>,
    /// Register names in thread order.
    pub reg_names: Vec<&'static str>,
    /// Memory dump rows.
    pub dump_rows: Vec<HexRow>,
    /// Debugger panels.
    pub panels: Vec<nerust_core_traits::debugger::DebugPanel>,
    /// Disassembly lines at the anchor.
    pub disasm: Vec<DisasmLine>,
    /// Program-counter anchor (`None` means no anchor).
    pub anchor: Option<u32>,
}

/// Split one thread batch into render-ready sections.
/// `None` on inspect failure: the caller serves degenerates, like the blocking drain.
pub(crate) fn assemble_snapshot_parts(batch: SnapshotResult) -> Option<SnapshotParts> {
    let inspected = batch.inspect.ok()?;
    Some(SnapshotParts {
        regs: inspected
            .registers
            .iter()
            .map(|&(name, value)| (name, value))
            .collect(),
        reg_names: inspected.registers.iter().map(|(name, _)| *name).collect(),
        dump_rows: inspected.dump.rows.iter().copied().collect(),
        panels: inspected.panels.iter().cloned().collect(),
        disasm: batch.disasm,
        anchor: batch.anchor,
    })
}

/// Watch-byte fold: the value at `addr` when the page row covers it.
/// Shared by the blocking snapshot and the drain flight.
pub(crate) fn fold_watch_byte(rows: &[HexRow], addr: u32) -> Option<u8> {
    rows.iter()
        .find(|row| addr >= row.addr && addr < row.addr + u32::from(row.valid))
        .map(|row| row.bytes[(addr - row.addr) as usize])
}

#[cfg(test)]
mod debug_snapshot_cov_tests {
    use crate::session::debug_snapshot::*;

    fn row(addr: u32, bytes: [u8; 16], valid: u8) -> HexRow {
        HexRow { addr, bytes, valid }
    }

    #[test]
    fn assemble_rejects_failed_inspect() {
        use nerust_core_traits::debugger::{InspectError, SnapshotResult};
        let batch = SnapshotResult {
            inspect: Err(InspectError::NotPaused),
            disasm: vec![],
            anchor: None,
        };
        assert!(assemble_snapshot_parts(batch).is_none());
    }

    #[test]
    fn fold_returns_byte_only_inside_valid_span() {
        let rows = [row(0x10, [9; 16], 4)];
        assert_eq!(fold_watch_byte(&rows, 0x12), Some(9));
        assert_eq!(fold_watch_byte(&rows, 0x0F), None);
        assert_eq!(fold_watch_byte(&rows, 0x14), None);
        assert_eq!(fold_watch_byte(&[], 0x10), None);
    }
}
