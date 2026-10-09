//! Debugger view-model: descriptor interpreter plus structured cache.
//!
//! Toolkit-independent: frontends render from snapshots and report
//! intents back. Movement state ([`NavState`]) and display data
//! ([`DisplayCache`]) are owned separately; navigation never reaches
//! into display locks and view code never parses kernel text.
//! Execution and writes stay in the shell session (later phases);
//! this module owns pure interpretation only.

use nerust_core_traits::debugger::DisasmLine;

/// Disassembly undo depth (no$ Back adapted to buttons).
pub const DISASM_BACK_CAP: usize = 32;

/// Bytes per hex-dump row in all debugger views.
pub const DUMP_ROW_BYTES: u32 = 16;

/// Movement state: spaces, addresses, inputs, undo stack, follow mode.
/// Presentation-owned; the session never sees it.
#[derive(Debug, Clone)]
pub struct NavState {
    space_idx: usize,
    space_count: usize,
    mem_addr: u32,
    mem_input: String,
    dis_addr: u32,
    dis_input: String,
    back: Vec<u32>,
    follow_pc: bool,
}

impl NavState {
    /// New state over `space_count` spaces, first space selected at
    /// `mem_start`.
    pub fn new(space_count: usize, mem_start: u32) -> Self {
        Self {
            space_idx: 0,
            space_count,
            mem_addr: mem_start,
            mem_input: format!("{mem_start:08X}"),
            dis_addr: 0,
            dis_input: String::new(),
            back: Vec::new(),
            follow_pc: true,
        }
    }

    pub fn space_idx(&self) -> usize {
        self.space_idx
    }

    pub fn mem_addr(&self) -> u32 {
        self.mem_addr
    }

    pub fn mem_input(&self) -> &str {
        &self.mem_input
    }

    pub fn dis_addr(&self) -> Option<u32> {
        // Follow mode leaves the pin stale; callers center on the PC.
        (!self.follow_pc).then_some(self.dis_addr)
    }

    pub fn dis_input(&self) -> &str {
        &self.dis_input
    }

    pub fn follow_pc(&self) -> bool {
        self.follow_pc
    }

    /// Undo depth for the Back label.
    pub fn back_len(&self) -> usize {
        self.back.len()
    }

    pub fn set_mem_input(&mut self, text: String) {
        self.mem_input = text;
    }

    pub fn set_dis_input(&mut self, text: String) {
        self.dis_input = text;
    }

    /// Jump the memory view; keeps the typed input in sync.
    pub fn go_mem(&mut self, addr: u32) {
        self.mem_addr = addr;
        self.mem_input = format!("{addr:08X}");
    }

    /// Page the memory view by `delta` bytes (positive or negative,
    /// saturating at zero).
    pub fn page_mem(&mut self, delta: i32) {
        let next = self.mem_addr.saturating_add_signed(delta);
        self.go_mem(next);
    }

    /// Switch space: selection resets to the space start.
    pub fn set_space(&mut self, idx: usize, start: u32) {
        if idx < self.space_count {
            self.space_idx = idx;
        }
        self.go_mem(start);
    }

    /// Pin `next`, pushing `current` for Back undo (cap 32, no
    /// consecutive duplicates). `current` is the displayed anchor:
    /// callers in follow-PC mode pass the first visible line, since
    /// the stored pin is stale there.
    pub fn navigate(&mut self, current: u32, next: u32) {
        if self.back.last() != Some(&current) {
            self.back.push(current);
            if self.back.len() > DISASM_BACK_CAP {
                self.back.remove(0);
            }
        }
        self.dis_addr = next;
        self.dis_input = format!("{next:08X}");
        self.follow_pc = false;
    }

    /// Pop the undo stack back onto the pin. `None` when empty.
    pub fn go_back(&mut self) -> Option<u32> {
        let addr = self.back.pop()?;
        self.dis_addr = addr;
        self.dis_input = format!("{addr:08X}");
        self.follow_pc = false;
        Some(addr)
    }

    /// Re-enable follow-PC mode.
    pub fn follow(&mut self) {
        self.follow_pc = true;
    }
}

/// One rebuild's worth of display data. Written by the drain, read by
/// view code; widgets never lock session state mid-build.
#[derive(Debug, Clone, Default)]
pub struct DisplayCache {
    /// `name: $xxxx` lines in kernel order.
    pub regs: String,
    /// `(address, text)` hex rows for the current page.
    pub dump_rows: Vec<(u32, String)>,
    /// Changed row addresses since the previous re-read (`*` marks).
    pub diff: Vec<u32>,
    /// Current disassembly window rows.
    pub disasm_lines: Vec<DisasmLine>,
    /// system panels text (`== label ==` sections).
    pub panels: String,
    /// `(label, width, height, rgba)` system images.
    pub images: Vec<(String, u32, u32, Vec<u8>)>,
    /// Read-only live watch value, if watched and readable.
    pub watch_value: Option<u8>,
    /// Status line (last outcome, or paused/running).
    pub status: String,
    /// Confirm-row text for a fully staged write, if any.
    pub pending_text: Option<String>,
}

/// Changed row addresses: rows whose text differs from the previous
/// re-read at the same address. Added rows (address absent before)
/// are navigation, not change, and stay unmarked.
pub fn diff_rows(prev: &[(u32, String)], next: &[(u32, String)]) -> Vec<u32> {
    next.iter()
        .filter(|(addr, text)| {
            prev.iter()
                .any(|(prev_addr, prev_text)| prev_addr == addr && prev_text != text)
        })
        .map(|(addr, _)| *addr)
        .collect()
}

/// Confirm-row text shared by the optimistic display and the
/// authoritative drain value. Single formatter: both writers call it.
pub fn format_pending_write(addr: u32, old: Option<u8>, value: u64) -> String {
    let old_text = match old {
        Some(old) => format!("{old:02X}"),
        None => "??".to_string(),
    };
    format!("write {value:02X} to {addr:08X} (was {old_text})?")
}

/// Parse a hex address (`0010`, `0x0010`, `$0010`). Surrounding
/// whitespace is ignored; anything else is `None`.
pub fn parse_hex_addr(raw: &str) -> Option<u32> {
    let text = raw.trim();
    let text = text.strip_prefix("0x").unwrap_or(text);
    let text = text.strip_prefix('$').unwrap_or(text);
    if text.is_empty() {
        return None;
    }
    u32::from_str_radix(text, 16).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn navigate_pins_and_pushes_anchor() {
        let mut nav = NavState::new(4, 0);
        assert!(nav.follow_pc());
        assert_eq!(nav.dis_addr(), None);
        nav.navigate(0xC03A, 0xC034);
        assert!(!nav.follow_pc());
        assert_eq!(nav.dis_addr(), Some(0xC034));
        assert_eq!(nav.dis_input(), "0000C034");
        assert_eq!(nav.back_len(), 1);
    }

    #[test]
    fn navigate_caps_and_dedups() {
        let mut nav = NavState::new(1, 0);
        for i in 0..(DISASM_BACK_CAP as u32 + 5) {
            nav.navigate(0xC000 + i, 0xC100 + i);
        }
        assert_eq!(nav.back_len(), DISASM_BACK_CAP);
        assert_eq!(nav.dis_addr(), Some(0xC100 + DISASM_BACK_CAP as u32 + 4));
        // Consecutive duplicates never stack: same anchor twice.
        let mut dup = NavState::new(1, 0);
        dup.navigate(0xC03A, 0xC034);
        dup.navigate(0xC03A, 0xC034);
        assert_eq!(dup.back_len(), 1);
    }

    #[test]
    fn go_back_pops_to_pin() {
        let mut nav = NavState::new(1, 0);
        assert_eq!(nav.go_back(), None);
        nav.navigate(0xC03A, 0xC034);
        nav.navigate(0xC034, 0xC000);
        assert_eq!(nav.go_back(), Some(0xC034));
        assert_eq!(nav.dis_addr(), Some(0xC034));
        assert_eq!(nav.go_back(), Some(0xC03A));
        assert_eq!(nav.go_back(), None);
        assert_eq!(nav.back_len(), 0);
    }

    #[test]
    fn space_switch_resets_to_start() {
        let mut nav = NavState::new(4, 0);
        nav.go_mem(0x0100);
        nav.set_space(1, 0x2000);
        assert_eq!(nav.space_idx(), 1);
        assert_eq!(nav.mem_addr(), 0x2000);
        assert_eq!(nav.mem_input(), "00002000");
        // Out-of-range index keeps the current space.
        nav.set_space(9, 0);
        assert_eq!(nav.space_idx(), 1);
    }

    #[test]
    fn page_mem_moves_by_delta() {
        let mut nav = NavState::new(1, 0x0100);
        nav.page_mem(0x80);
        assert_eq!(nav.mem_addr(), 0x0180);
        nav.page_mem(-0x100);
        assert_eq!(nav.mem_addr(), 0x0080);
        nav.page_mem(-0x1000);
        assert_eq!(nav.mem_addr(), 0);
    }

    #[test]
    fn follow_reenables_pc_mode() {
        let mut nav = NavState::new(1, 0);
        nav.navigate(0xC03A, 0xC034);
        nav.follow();
        assert!(nav.follow_pc());
        assert_eq!(nav.dis_addr(), None);
    }

    #[test]
    fn diff_rows_marks_changed_only() {
        let prev = vec![(0u32, "00: 00".to_string()), (16u32, "10: 00".to_string())];
        let next = vec![
            (0u32, "00: 42".to_string()),
            (16u32, "10: 00".to_string()),
            (32u32, "20: 00".to_string()),
        ];
        // Changed row marks; unchanged and added rows do not.
        assert_eq!(diff_rows(&prev, &next), vec![0]);
        assert!(diff_rows(&prev, &prev).is_empty());
    }

    #[test]
    fn format_pending_write_covers_unknown_old() {
        assert_eq!(
            format_pending_write(0, Some(0xAB), 0x42),
            "write 42 to 00000000 (was AB)?"
        );
        assert_eq!(
            format_pending_write(0x80, None, 0xCC),
            "write CC to 00000080 (was ??)?"
        );
    }

    #[test]
    fn parse_hex_addr_accepts_plain_0x_and_dollar() {
        assert_eq!(parse_hex_addr("0010"), Some(0x10));
        assert_eq!(parse_hex_addr("0x0010"), Some(0x10));
        assert_eq!(parse_hex_addr("$0010"), Some(0x10));
        assert_eq!(parse_hex_addr("  $C03A  "), Some(0xC03A));
        assert_eq!(parse_hex_addr("FFFFFFFF"), Some(0xFFFF_FFFF));
        assert_eq!(parse_hex_addr(""), None);
        assert_eq!(parse_hex_addr("zz"), None);
        assert_eq!(parse_hex_addr("0x"), None);
        assert_eq!(parse_hex_addr("$"), None);
        assert_eq!(parse_hex_addr("1FFFFFFFF"), None);
        assert_eq!(parse_hex_addr("-1"), None);
    }

    #[test]
    fn display_cache_covers_all_sections() {
        // Snapshot shape: every section the drain refreshes has a slot.
        let cache = DisplayCache {
            regs: "a: $00".to_string(),
            dump_rows: vec![(0, "00000000: 00".to_string())],
            diff: vec![0],
            disasm_lines: vec![DisasmLine {
                addr: 0xC000,
                bytes: [0xEA, 0, 0],
                len: 1,
                text: "NOP".to_string(),
                is_pc: true,
                target: None,
            }],
            panels: "== P ==".to_string(),
            images: vec![("Pattern left".to_string(), 256, 256, vec![0xFF; 4])],
            watch_value: Some(0x42),
            status: "paused".to_string(),
            pending_text: Some("write 42 to 00000000 (was 00)?".to_string()),
        };
        assert_eq!(cache.dump_rows.len(), 1);
        assert_eq!(cache.disasm_lines[0].addr, 0xC000);
        assert_eq!(cache.images.len(), 1);
    }
}
