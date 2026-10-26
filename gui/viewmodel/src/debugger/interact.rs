//! Debugger interaction rules: anchor derivation, row markers,
//! bookmark list policy, and value validation. Pure; both viewers call
//! these so the same gesture reads identically everywhere.

use super::nav::parse_hex_addr;

/// Bookmark table cap shared by every frontend.
pub const BOOKMARK_CAP: usize = 8;

/// Displayed disassembly anchor: first visible line in follow-PC mode
/// (the stored pin is stale there), else the pin.
pub fn anchor(follow_pc: bool, first_line_addr: Option<u32>, dis_addr: Option<u32>) -> u32 {
    if follow_pc {
        first_line_addr.unwrap_or(0)
    } else {
        dis_addr.unwrap_or(0)
    }
}

/// Containing-row match: a mid-row pick still lights its row.
pub fn row_hit(selected_addr: Option<u32>, row_addr: u32, valid: usize) -> bool {
    selected_addr.is_some_and(|a| a >= row_addr && a < row_addr + valid as u32)
}

/// Dump marker from selection and diff state.
pub fn dump_marker(row_hit: bool, changed: bool) -> &'static str {
    match (row_hit, changed) {
        (true, true) => ">*",
        (true, false) => "> ",
        (false, true) => " *",
        (false, false) => "  ",
    }
}

/// Disassembly marker: the PC `>` already comes from
/// [`format_disasm_line`](super::format::format_disasm_line), so the PC
/// row contributes blanks here; only a pinned non-PC row carries `@`.
/// Prepending another `"> "` double-marks the PC row.
pub fn disasm_marker(is_pc: bool, is_pinned: bool) -> &'static str {
    if !is_pc && is_pinned {
        "@ "
    } else {
        "  "
    }
}

/// Empty bookmark labels default to `BM{n}` so Add always shows something.
pub fn default_bookmark_label(n: usize) -> String {
    format!("BM{n}")
}

/// Address already bookmarked: pushes dedup instead of doubling entries.
pub fn bookmark_contains(bookmarks: &[(u32, String)], addr: u32) -> bool {
    bookmarks.iter().any(|(a, _)| *a == addr)
}

/// Write/freeze value: hex with `0x`/`$`/bare prefixes, single byte only.
pub fn parse_byte_value(input: &str) -> Option<u8> {
    parse_hex_addr(input)
        .filter(|value| *value <= 0xFF)
        .map(|value| value as u8)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn anchor_prefers_first_line_in_follow_mode() {
        assert_eq!(anchor(true, Some(0x100), Some(0x200)), 0x100);
        assert_eq!(anchor(false, Some(0x100), Some(0x200)), 0x200);
        assert_eq!(anchor(true, None, None), 0);
    }

    #[test]
    fn dump_marker_covers_all_four_states() {
        assert_eq!(dump_marker(true, true), ">*");
        assert_eq!(dump_marker(true, false), "> ");
        assert_eq!(dump_marker(false, true), " *");
        assert_eq!(dump_marker(false, false), "  ");
    }

    #[test]
    fn byte_value_rejects_out_of_range() {
        assert_eq!(parse_byte_value("ff"), Some(0xFF));
        assert_eq!(parse_byte_value("0x100"), None);
        assert_eq!(parse_byte_value("zz"), None);
    }
}

#[cfg(test)]
mod marker_composition_tests {
    use super::*;
    use crate::debugger::format::format_disasm_line;
    use nerust_core_traits::debugger::DisasmLine;

    #[test]
    fn disasm_marker_never_double_marks_pc() {
        let pc_line = DisasmLine {
            addr: 0xC293,
            bytes: [0x60, 0, 0, 0],
            len: 1,
            text: "RTS".to_string(),
            is_pc: true,
            target: None,
        };
        // The composed row carries exactly one `>`: the format owns
        // it, the shared prefix contributes blanks.
        let composed = format!(
            "{}{}",
            disasm_marker(pc_line.is_pc, true),
            format_disasm_line(&pc_line)
        );
        assert!(composed.starts_with("  >0x"), "got {composed:?}");
        assert_eq!(disasm_marker(false, true), "@ ");
        assert_eq!(disasm_marker(false, false), "  ");
        assert_eq!(disasm_marker(true, false), "  ");
    }
}
