//! Pure text formatters for debugger sections. Kernel order
//! kept; wording-free (status wording lives in `status`).

use nerust_core_traits::debugger::{CellValue, ColumnKind, DebugPanel, DisasmLine};

/// Empty-panels marker. Frontends hide the Panels section on
/// this instead of showing a puzzling label.
pub const EMPTY_PANELS_TEXT: &str = "(no panels)";

/// Whether a formatted panels string carries content.
pub fn panels_visible(panels: &str) -> bool {
    panels != EMPTY_PANELS_TEXT
}

/// Format one kernel register line (`name: 0xxxx`), kernel order
/// kept. The `0x` canon is cross-system: `$` is NES-only nostalgia
/// and this debugger serves NES, GBC, and GBA alike.
pub fn format_register(name: &'static str, value: u64) -> String {
    format!("{name}: 0x{value:04X}")
}

/// Format one cell by its column kind.
pub fn format_cell(cell: &CellValue, kind: ColumnKind) -> String {
    match (cell, kind) {
        (CellValue::Text(text), _) => text.clone(),
        (CellValue::U64(value), ColumnKind::Hex { digits }) => {
            format!("{:01$X}", value, digits as usize)
        }
        (CellValue::U64(value), ColumnKind::Dec) => format!("{value}"),
        (CellValue::U64(value), _) => format!("{value}"),
        (CellValue::Bool(true), ColumnKind::Bool) => "yes".to_string(),
        (CellValue::Bool(false), ColumnKind::Bool) => "no".to_string(),
        (CellValue::Bool(value), _) => format!("{value}"),
    }
}

/// Format panels as `== label ==` / `key: cells` text, or the
/// degenerate marker when empty.
pub fn format_panels(panels: &[DebugPanel]) -> String {
    if panels.is_empty() {
        return EMPTY_PANELS_TEXT.to_string();
    }
    use std::fmt::Write as _;
    let mut out = String::new();
    for panel in panels.iter() {
        let _ = writeln!(out, "== {} ==", panel.label_id);
        for row in panel.rows.iter() {
            let _ = write!(out, "{}:", row.key);
            for (cell, column) in row.cells.iter().zip(panel.columns.iter()) {
                let _ = write!(out, " {}", format_cell(cell, column.kind));
            }
            out.push('\n');
        }
    }
    out
}

/// Format one disassembly row (`>0xaddr: bytes  text`) for line
/// buttons. The PC row carries the `>` mark (Mesen canon); the byte
/// field pads to 4 bytes so mnemonics align across rows.
pub fn format_disasm_line(row: &DisasmLine) -> String {
    use std::fmt::Write as _;
    let mut out = String::new();
    let mark = if row.is_pc { '>' } else { ' ' };
    let _ = write!(out, "{mark}0x{:08X}: ", row.addr);
    for byte in row.bytes.iter().take(row.len as usize) {
        let _ = write!(out, "{byte:02X} ");
    }
    for _ in row.len..4 {
        out.push_str("   ");
    }
    out.push_str(&row.text);
    out
}
