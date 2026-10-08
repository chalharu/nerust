//! SPIKE-ONLY throwaway prototype. Do not merge into any long-lived branch.
//!
//! Generic interpreter for Memory / Registers / Table view descriptors.
//! Mirrors `debugger-ui-design.md` §1: the frontend renders what this
//! module produces, with no per-system branches. The Table input here is
//! a STAND-IN (`standin_cpu_table`): NES `panels()` is currently empty,
//! and the spike is forbidden from changing kernel code, so the CPU
//! status table is synthesized from registers. Production Table
//! descriptors must come from the core.

use nerust_core_traits::debugger::{CellValue, DebugPanel, MemoryDump, PanelColumn, PanelRow};

/// Rendered debug data. Plain strings: the tao/GTK renderers display
/// these verbatim and own no formatting logic.
#[derive(Debug, Clone, Default)]
pub struct DebugViewData {
    pub space_names: Vec<String>,
    pub selected_space: usize,
    pub hex_lines: Vec<String>,
    pub register_lines: Vec<String>,
    pub table_title: String,
    pub table_lines: Vec<String>,
    pub table_is_standin: bool,
    pub paused: bool,
    pub frame: u64,
    pub error: Option<String>,
    pub refresh_micros: u128,
}

/// Format `MemoryDump` rows as `ADDR: b0 b1 ...` lines, honoring each
/// row's `valid` count so a short final row is distinguishable.
pub fn hex_lines(dump: &MemoryDump) -> Vec<String> {
    dump.rows
        .iter()
        .map(|row| {
            let bytes: Vec<String> = row.bytes[..row.valid as usize]
                .iter()
                .map(|b| format!("{b:02X}"))
                .collect();
            format!("{:04X}: {}", row.addr, bytes.join(" "))
        })
        .collect()
}

/// Format registers in `registers()` order. Order is a core contract;
/// the frontend never sorts.
pub fn register_lines(registers: &[(&'static str, u64)]) -> Vec<String> {
    registers
        .iter()
        .map(|(name, value)| format!("{name} = {value:#X}"))
        .collect()
}

/// Format a `DebugPanel` table: header from column labels, one line per row.
pub fn table_text(panel: &DebugPanel) -> (String, Vec<String>) {
    let header: Vec<&str> = panel.columns.iter().map(|c| c.label_id).collect();
    let lines = panel
        .rows
        .iter()
        .map(|row| {
            let cells: Vec<String> = row.cells.iter().map(cell_text).collect();
            format!("{}: {}", row.key, cells.join(" | "))
        })
        .collect();
    (format!("{}: {}", panel.label_id, header.join(" | ")), lines)
}

fn cell_text(cell: &CellValue) -> String {
    match cell {
        CellValue::Text(s) => s.clone(),
        CellValue::U64(v) => format!("{v:#X}"),
        CellValue::Bool(b) => b.to_string(),
    }
}

/// STAND-IN Table descriptor synthesized from registers.
///
/// NES `panels()` returns no panels, so the spike builds a CPU status
/// table here to exercise the generic Table renderer end to end.
/// Production code must delete this: Table descriptors are core-owned.
pub fn standin_cpu_table(registers: &[(&'static str, u64)]) -> DebugPanel {
    static COLUMNS: [PanelColumn; 2] = [
        PanelColumn {
            id: "name",
            label_id: "register",
            kind: nerust_core_traits::debugger::ColumnKind::Text,
        },
        PanelColumn {
            id: "value",
            label_id: "value",
            kind: nerust_core_traits::debugger::ColumnKind::Hex { digits: 4 },
        },
    ];
    DebugPanel {
        id: "spike-standin-cpu",
        label_id: "CPU (spike stand-in)",
        columns: &COLUMNS,
        rows: registers
            .iter()
            .map(|(name, value)| PanelRow {
                key: name,
                cells: vec![CellValue::Text(name.to_string()), CellValue::U64(*value)],
            })
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use nerust_core_traits::debugger::{DUMP_ROW_BYTES, HexRow, SpaceId};

    use super::*;

    fn sample_dump() -> MemoryDump {
        let mut bytes = [0u8; DUMP_ROW_BYTES];
        bytes[0] = 0xDE;
        bytes[1] = 0xAD;
        MemoryDump {
            space: SpaceId(0),
            base: 0x8000,
            rows: Arc::new([HexRow {
                addr: 0x8000,
                valid: 2,
                bytes,
            }]),
        }
    }

    #[test]
    fn hex_lines_honor_valid_count() {
        assert_eq!(hex_lines(&sample_dump()), vec!["8000: DE AD"]);
    }

    #[test]
    fn register_lines_preserve_order() {
        let regs = [("b", 2), ("a", 1)];
        assert_eq!(register_lines(&regs), vec!["b = 0x2", "a = 0x1"]);
    }

    #[test]
    fn standin_table_marks_itself() {
        let panel = standin_cpu_table(&[("a", 1)]);
        assert_eq!(panel.id, "spike-standin-cpu");
        assert_eq!(panel.rows.len(), 1);
        let (title, lines) = table_text(&panel);
        assert!(title.contains("spike stand-in"));
        assert_eq!(lines.len(), 1);
    }
}
