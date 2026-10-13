//! Debugger view-model: descriptor interpreter plus structured cache.
//!
//! Toolkit-independent: frontends render from snapshots and report
//! intents back. Movement state ([`NavState`]) and display data
//! ([`DisplayCache`]) are owned separately; navigation never reaches
//! into display locks and view code never parses kernel text.
//! Execution and writes stay in the shell session (later phases);
//! this module owns pure interpretation only.

use nerust_core_traits::debugger::{
    CellValue, ColumnKind, DebugImage, DebugPanel, DebuggerError, DisasmLine, HexRow, ImageFormat,
    SpaceId, SpaceTable,
};
use nerust_gui_settings::language::AppLanguage;

/// Disassembly undo depth (no$ Back adapted to buttons).
pub const DISASM_BACK_CAP: usize = 32;

/// PPU viewer empty-hover hint. Canonical wording for every
/// frontend; PPU image titles render the raw kernel label.
pub const PPU_HOVER_HINT: &str = "hover a tile";

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
            mem_input: format!("0x{mem_start:08X}"),
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
        self.mem_input = format!("0x{addr:08X}");
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

    /// Cycle spaces with wrap-around (space stepper). Returns the new
    /// index, or `None` when there are no spaces.
    pub fn cycle_space(&mut self, delta: i32) -> Option<usize> {
        if self.space_count == 0 {
            return None;
        }
        let len = self.space_count as i32;
        let next = (self.space_idx as i32 + delta).rem_euclid(len) as usize;
        self.space_idx = next;
        Some(next)
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
        self.dis_input = format!("0x{next:08X}");
        self.follow_pc = false;
    }

    /// Pop the undo stack back onto the pin. `None` when empty.
    pub fn go_back(&mut self) -> Option<u32> {
        let addr = self.back.pop()?;
        self.dis_addr = addr;
        self.dis_input = format!("0x{addr:08X}");
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
///
/// Registers, dump rows, and panels travel structured (kernel order
/// kept): formatting to text is presentation's job, done per rebuild
/// with the `format_*` helpers below. Outcomes travel structured too
/// ([`StatusMessage`], [`PendingWrite`]: the session reports *what*
/// happened, frontends render the wording in their own language.
#[derive(Debug, Clone, Default)]
pub struct DisplayCache {
    /// `(name, value)` in kernel order.
    pub regs: Vec<(&'static str, u64)>,
    /// Raw hex rows for the current scroll window.
    pub dump_rows: Vec<HexRow>,
    /// Changed row addresses since the previous re-read (`*` marks).
    pub diff: Vec<u32>,
    /// Current disassembly window rows.
    pub disasm_lines: Vec<DisasmLine>,
    /// System observation tables (empty when the core has none).
    pub panels: Vec<DebugPanel>,
    /// `(label, width, height, rgba)` system images.
    pub images: Vec<(String, u32, u32, Vec<u8>)>,
    /// Image source generation behind `images` (see
    /// `DebugSnapshot::images_gen`).
    pub images_gen: u64,
    /// Read-only live watch value, if watched and readable.
    pub watch_value: Option<u8>,
    /// Pause state at capture: drives the single Pause/Resume toggle
    /// label without scraping status text.
    pub paused: bool,
    /// Register names parallel to `regs` lines, for the register
    /// viewer target. Kernel `&'static str`: no allocation, and the
    /// drain's write path reuses the exact names.
    pub reg_names: Vec<&'static str>,
    /// Status line (last outcome, or paused/running), rendered by the
    /// frontend in its own language.
    pub status: StatusMessage,
    /// Staged write awaiting confirm, if any. The frontend renders
    /// the confirm row from this.
    pub pending: Option<PendingWrite>,
}

impl DisplayCache {
    /// Clone everything but the system images. The debugger view
    /// never renders images (the PPU viewer owns them through its
    /// own Sync cache), so dragging up to 512KB of RGBA through
    /// every rebuild is pure waste.
    pub fn clone_without_images(&self) -> Self {
        Self {
            regs: self.regs.clone(),
            dump_rows: self.dump_rows.clone(),
            diff: self.diff.clone(),
            disasm_lines: self.disasm_lines.clone(),
            panels: self.panels.clone(),
            images: Vec::new(),
            images_gen: self.images_gen,
            watch_value: self.watch_value,
            paused: self.paused,
            reg_names: self.reg_names.clone(),
            status: self.status.clone(),
            pending: self.pending,
        }
    }

    /// Restore the last paused sections over a running drain. The
    /// paused-gated inspect and disassembly refuse while the core
    /// runs, so Resume (or Refresh while running) would otherwise
    /// blank the dump, registers, and disassembly the user paused on.
    /// Images, status, and the pause chrome already carry fresh values
    /// and stay untouched: the rows read as last-paused under a
    /// "running" status, which is honest (no frame can be pinned while
    /// running).
    pub fn restore_paused_sections(&mut self, old: &DisplayCache) {
        if self.paused {
            return;
        }
        self.dump_rows.clone_from(&old.dump_rows);
        self.regs.clone_from(&old.regs);
        self.reg_names.clone_from(&old.reg_names);
        self.disasm_lines.clone_from(&old.disasm_lines);
        self.panels.clone_from(&old.panels);
        self.diff.clone_from(&old.diff);
    }
}

/// One debugger outcome, data only. The session and drain report
/// *what* happened; frontends render the wording via
/// [`StatusMessage::render`] in their own language. This keeps
/// presentation (wording, language) out of the application layer:
/// the drain no longer needs a language, and both languages are
/// covered by construction.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum StatusMessage {
    /// Nothing to report yet (empty display).
    #[default]
    Empty,
    Paused,
    Running,
    Resume,
    NothingToWrite,
    NoCore,
    Stepped(u64),
    WroteMemory {
        addr: u32,
        value: u64,
    },
    RegisterWrote {
        name: &'static str,
        value: u64,
    },
    PauseFailed(String),
    ResumeFailed(String),
    StepFailed(String),
    WriteRefused(String),
    RegisterRefused(String),
    ThreadFailed(String),
    /// Frontend-local wording (parse errors, selection prompts):
    /// never produced by the session, rendered verbatim.
    Local(String),
}

impl StatusMessage {
    /// Render the outcome in a language. Single wording shared by
    /// every frontend (and its tests).
    pub fn render(&self, language: AppLanguage) -> String {
        use nerust_settings_core::i18n as ui_msg;
        match self {
            Self::Empty => String::new(),
            Self::Paused => ui_msg::text(language, ui_msg::UiText::PausedState).to_string(),
            Self::Running => ui_msg::text(language, ui_msg::UiText::RunningState).to_string(),
            Self::Resume => ui_msg::text(language, ui_msg::UiText::Resume).to_string(),
            Self::NothingToWrite => {
                ui_msg::text(language, ui_msg::UiText::NothingToWrite).to_string()
            }
            Self::NoCore => ui_msg::text(language, ui_msg::UiText::NoCore).to_string(),
            Self::Stepped(count) => ui_msg::stepped(language, *count),
            Self::WroteMemory { addr, value } => ui_msg::wrote_memory(language, *addr, *value),
            Self::RegisterWrote { name, value } => ui_msg::register_wrote(language, name, *value),
            Self::PauseFailed(detail) => ui_msg::pause_failed(language, detail),
            Self::ResumeFailed(detail) => ui_msg::resume_failed(language, detail),
            Self::StepFailed(detail) => ui_msg::step_failed(language, detail),
            Self::WriteRefused(detail) => ui_msg::write_refused(language, detail),
            Self::RegisterRefused(detail) => ui_msg::register_refused(language, detail),
            Self::ThreadFailed(detail) => ui_msg::thread_failed(language, detail),
            Self::Local(text) => text.clone(),
        }
    }
}

/// A fully staged write awaiting confirm. Rendered by the frontend's
/// confirm row; the drain and any optimistic display share this one
/// shape instead of pre-formatted strings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PendingWrite {
    pub addr: u32,
    pub old: Option<u8>,
    pub value: u64,
}

impl PendingWrite {
    /// Confirm-row text in a language. Single formatter shared by
    /// every frontend.
    pub fn render(&self, language: AppLanguage) -> String {
        nerust_settings_core::i18n::pending_write(language, self.addr, self.old, self.value)
    }
}

/// Resolve the display key for a [`DebuggerError`]. `None` for
/// variants without a table entry. Lives outside the kernel: error
/// display is presentation's duty, so `UnknownSpace` never
/// fabricates a key from inside domain code.
pub fn space_key(table: &SpaceTable, error: &DebuggerError) -> Option<&'static str> {
    let id: SpaceId = match error {
        DebuggerError::UnknownSpace(id) | DebuggerError::ReadOnlySpace(id) => *id,
        DebuggerError::UnmappedAddress { space, .. } => *space,
        _ => return None,
    };
    table.get(id).map(|s| s.key)
}

/// Changed row addresses: rows whose bytes differ from the previous
/// re-read at the same address. Added rows (address absent before)
/// are navigation, not change, and stay unmarked.
pub fn diff_rows(prev: &[HexRow], next: &[HexRow]) -> Vec<u32> {
    next.iter()
        .filter(|row| {
            prev.iter().any(|p| {
                p.addr == row.addr
                    && (p.valid != row.valid
                        || p.bytes[..p.valid as usize] != row.bytes[..row.valid as usize])
            })
        })
        .map(|row| row.addr)
        .collect()
}

/// Two-phase memory write transaction: `Empty` → `Prepared` (row
/// selected, old byte read) → `Staged` (value parsed) → commit
/// consumes. Pure transitions; the session owns one instance and
/// applies the stale-kill rule around drains.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum WriteTransaction {
    /// No pending write.
    #[default]
    Empty,
    /// Row selected; value not yet staged.
    Prepared {
        space: SpaceId,
        addr: u32,
        old: Option<u8>,
    },
    /// Fully staged; the confirm row renders [`PendingWrite`].
    Staged {
        space: SpaceId,
        addr: u32,
        old: Option<u8>,
        value: u64,
    },
}

impl WriteTransaction {
    /// Fresh selection overwrites any state.
    pub fn prepare(&mut self, space: SpaceId, addr: u32, old: Option<u8>) {
        *self = Self::Prepared { space, addr, old };
    }

    /// Stage the parsed value; only `Prepared` accepts it.
    pub fn stage(&mut self, value: u64) {
        if let Self::Prepared { space, addr, old } = *self {
            *self = Self::Staged {
                space,
                addr,
                old,
                value,
            };
        }
    }

    /// Commit the staged write, consuming the transaction. `None`
    /// unless fully staged.
    pub fn take_commit(&mut self) -> Option<(SpaceId, u32, u64)> {
        match *self {
            Self::Staged {
                space, addr, value, ..
            } => {
                *self = Self::Empty;
                Some((space, addr, value))
            }
            _ => None,
        }
    }

    /// Drop any uncommitted transaction (cancel, space switch).
    pub fn clear(&mut self) {
        *self = Self::Empty;
    }

    /// State-changing traffic (step, refresh, pause switches) kills
    /// staged-but-uncommitted values from previous drains, whose old
    /// byte may no longer match. Select/stage/commit traffic never
    /// triggers this: it builds the transaction this drain consumes.
    pub fn on_state_changing_traffic(&mut self) {
        if !matches!(self, Self::Empty) {
            *self = Self::Empty;
        }
    }

    /// Staged write, if fully staged. The frontend renders the
    /// confirm row from this (no language here: presentation owns it).
    pub fn pending(&self) -> Option<PendingWrite> {
        match *self {
            Self::Staged {
                addr, old, value, ..
            } => Some(PendingWrite { addr, old, value }),
            _ => None,
        }
    }
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

/// One drain's worth of interpreted debugger data. The session fills
/// it from a single batched core pass; frontends render from it and
/// never call the core themselves.
#[derive(Debug, Clone, Default)]
pub struct DebugSnapshot {
    /// False when no core is loaded: all sections carry degenerates.
    pub available: bool,
    /// `(name, value)` in kernel order.
    pub regs: Vec<(&'static str, u64)>,
    /// Raw hex rows for the current scroll window.
    pub dump_rows: Vec<HexRow>,
    /// Current disassembly window rows (empty when unanchored).
    pub disasm_lines: Vec<DisasmLine>,
    /// Anchor the disassembly centers on (`None` in follow mode
    /// without a named counter, or when pinned explicitly elsewhere).
    pub pc: Option<u32>,
    /// System observation tables (empty when the core has none).
    pub panels: Vec<DebugPanel>,
    /// `(label, width, height, rgba)` system images.
    pub images: Vec<(String, u32, u32, Vec<u8>)>,
    /// Image source generation: bumped by every path that advances
    /// or replaces memory, so PPU viewers skip re-deriving on
    /// identical generations instead of hashing megabytes.
    pub images_gen: u64,
    /// Live watch value folded into the page read (`None` when
    /// unwatched or off-page; the drain falls back to one read).
    pub watch: Option<u8>,
    /// Register names parallel to `regs` lines.
    pub reg_names: Vec<&'static str>,
}

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

/// Blit an indexed system image to `(label, width, height, rgba)`.
/// The blit is mechanical (no system knowledge): indexed pixels
/// through the descriptor palette, opaque alpha.
pub fn rgba_image(image: &DebugImage) -> (String, u32, u32, Vec<u8>) {
    let mut rgba = Vec::with_capacity(image.pixels.len() * 4);
    for &pixel in &image.pixels {
        let rgb = match image.format {
            ImageFormat::Indexed2bpp | ImageFormat::Indexed4bpp => image
                .palette
                .get(pixel as usize)
                .copied()
                .unwrap_or([0, 0, 0]),
        };
        rgba.extend_from_slice(&[rgb[0], rgb[1], rgb[2], 0xFF]);
    }
    (image.label_id.to_string(), image.width, image.height, rgba)
}

/// Nearest-neighbor 2x scale of blitted RGBA bytes. Pure; frontends
/// share it so the PPU viewer shows identical pixels everywhere.
pub fn scale2x_nearest(rgba: &[u8], width: u32, height: u32) -> Vec<u8> {
    let (w, h) = (width as usize, height as usize);
    let mut out = vec![0u8; w * 2 * h * 2 * 4];
    for y in 0..h {
        for x in 0..w {
            let src = &rgba[(y * w + x) * 4..(y * w + x) * 4 + 4];
            for dy in 0..2 {
                for dx in 0..2 {
                    let dst = ((y * 2 + dy) * w * 2 + (x * 2 + dx)) * 4;
                    out[dst..dst + 4].copy_from_slice(src);
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cached(rows: bool) -> DisplayCache {
        DisplayCache {
            dump_rows: if rows {
                vec![HexRow {
                    addr: 0x10,
                    valid: 1,
                    bytes: [0xAB, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
                }]
            } else {
                Vec::new()
            },
            regs: if rows { vec![("a", 1)] } else { Vec::new() },
            status: StatusMessage::Paused,
            paused: true,
            ..DisplayCache::default()
        }
    }

    #[test]
    fn restore_keeps_paused_sections_over_running_drain() {
        let old = cached(true);
        let mut new = DisplayCache {
            paused: false,
            status: StatusMessage::Running,
            ..DisplayCache::default()
        };
        new.restore_paused_sections(&old);
        assert_eq!(new.dump_rows.len(), 1);
        assert_eq!(new.regs, vec![("a", 1)]);
        assert_eq!(new.status, StatusMessage::Running, "fresh chrome stays");
        // Paused drains pass through untouched.
        let mut paused_new = DisplayCache {
            paused: true,
            ..DisplayCache::default()
        };
        paused_new.restore_paused_sections(&old);
        assert!(paused_new.dump_rows.is_empty());
    }

    #[test]
    fn ppu_hover_hint_is_canonical() {
        // Single wording shared by the tao and GTK viewers.
        assert_eq!(PPU_HOVER_HINT, "hover a tile");
    }

    #[test]
    fn scale2x_doubles_each_pixel() {
        // 2x1 red/green becomes 4x2 blocks.
        let rgba = vec![255, 0, 0, 0xFF, 0, 255, 0, 0xFF];
        let out = scale2x_nearest(&rgba, 2, 1);
        assert_eq!(out.len(), 4 * 2 * 4);
        // Top row is red red green green.
        assert_eq!(&out[0..4], &[255, 0, 0, 0xFF]);
        assert_eq!(&out[4..8], &[255, 0, 0, 0xFF]);
        assert_eq!(&out[8..12], &[0, 255, 0, 0xFF]);
        assert_eq!(&out[12..16], &[0, 255, 0, 0xFF]);
        // Bottom row repeats the top row.
        assert_eq!(&out[16..20], &[255, 0, 0, 0xFF]);
        assert_eq!(&out[24..28], &[0, 255, 0, 0xFF]);
    }

    #[test]
    fn navigate_pins_and_pushes_anchor() {
        let mut nav = NavState::new(4, 0);
        assert!(nav.follow_pc());
        assert_eq!(nav.dis_addr(), None);
        nav.navigate(0xC03A, 0xC034);
        assert!(!nav.follow_pc());
        assert_eq!(nav.dis_addr(), Some(0xC034));
        assert_eq!(nav.dis_input(), "0x0000C034");
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
        assert_eq!(nav.mem_input(), "0x00002000");
        // Out-of-range index keeps the current space.
        nav.set_space(9, 0);
        assert_eq!(nav.space_idx(), 1);
    }

    #[test]
    fn cycle_space_wraps_around() {
        let mut nav = NavState::new(4, 0);
        assert_eq!(nav.cycle_space(1), Some(1));
        assert_eq!(nav.cycle_space(1), Some(2));
        assert_eq!(nav.cycle_space(-1), Some(1));
        assert_eq!(nav.cycle_space(-2), Some(3));
        assert_eq!(nav.cycle_space(1), Some(0));
        let mut empty = NavState::new(0, 0);
        assert_eq!(empty.cycle_space(1), None);
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
        use nerust_core_traits::debugger::HexRow;
        let row = |addr: u32, bytes: &[u8]| HexRow {
            addr,
            valid: bytes.len() as u8,
            bytes: {
                let mut all = [0u8; 16];
                all[..bytes.len()].copy_from_slice(bytes);
                all
            },
        };
        let prev = vec![row(0, &[0x00]), row(16, &[0x00])];
        let next = vec![row(0, &[0x42]), row(16, &[0x00]), row(32, &[0x00])];
        // Changed row marks; unchanged and added rows do not.
        assert_eq!(diff_rows(&prev, &next), vec![0]);
        assert!(diff_rows(&prev, &prev).is_empty());
    }

    #[test]
    fn pending_write_renders_in_both_languages() {
        use nerust_gui_settings::language::AppLanguage;
        let staged = PendingWrite {
            addr: 0,
            old: Some(0xAB),
            value: 0x42,
        };
        assert_eq!(
            staged.render(AppLanguage::English),
            "write 42 to 0x00000000 (was AB)?"
        );
        let unknown = PendingWrite {
            addr: 0x80,
            old: None,
            value: 0xCC,
        };
        assert_eq!(
            unknown.render(AppLanguage::English),
            "write CC to 0x00000080 (was ??)?"
        );
        assert_eq!(
            unknown.render(AppLanguage::Japanese),
            "0x00000080 に CC を書込 (元 ??)?"
        );
    }

    #[test]
    fn status_message_renders_every_variant() {
        use nerust_gui_settings::language::AppLanguage;
        let en = AppLanguage::English;
        assert_eq!(StatusMessage::Empty.render(en), "");
        assert_eq!(StatusMessage::Paused.render(en), "paused");
        assert_eq!(StatusMessage::Running.render(en), "running");
        assert_eq!(StatusMessage::Resume.render(en), "Resume");
        assert_eq!(StatusMessage::NothingToWrite.render(en), "nothing to write");
        assert_eq!(StatusMessage::NoCore.render(en), "no core");
        assert_eq!(StatusMessage::Stepped(7).render(en), "stepped 7");
        assert_eq!(
            StatusMessage::WroteMemory {
                addr: 0x10,
                value: 0x42
            }
            .render(en),
            "wrote 42 to 0x00000010 (width 1)"
        );
        assert_eq!(
            StatusMessage::RegisterWrote {
                name: "r7",
                value: 1
            }
            .render(en),
            "wrote 0x0001 to register r7"
        );
        assert_eq!(
            StatusMessage::WriteRefused("X".to_string()).render(en),
            "write refused: X"
        );
        assert_eq!(
            StatusMessage::Local("custom".to_string()).render(en),
            "custom"
        );
        // One Japanese spot-check pins the language threading.
        assert_eq!(
            StatusMessage::Paused.render(AppLanguage::Japanese),
            "一時停止中"
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
        use nerust_core_traits::debugger::HexRow;
        // Snapshot shape: every section the drain refreshes has a slot.
        let cache = DisplayCache {
            regs: vec![("a", 0)],
            dump_rows: vec![HexRow {
                addr: 0,
                valid: 1,
                bytes: [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
            }],
            diff: vec![0],
            disasm_lines: vec![DisasmLine {
                addr: 0xC000,
                bytes: [0xEA, 0, 0, 0],
                len: 1,
                text: "NOP".to_string(),
                is_pc: true,
                target: None,
            }],
            panels: vec![],
            images: vec![("Pattern left".to_string(), 256, 256, vec![0xFF; 4])],
            images_gen: 7,
            watch_value: Some(0x42),
            paused: true,
            reg_names: vec!["a"],
            status: StatusMessage::Paused,
            pending: Some(PendingWrite {
                addr: 0,
                old: Some(0x00),
                value: 0x42,
            }),
        };
        assert_eq!(cache.dump_rows.len(), 1);
        assert_eq!(cache.disasm_lines[0].addr, 0xC000);
        assert_eq!(cache.images.len(), 1);
        // The imageless clone keeps the generation: viewers compare
        // generations, never pixels.
        let light = cache.clone_without_images();
        assert!(light.images.is_empty());
        assert_eq!(light.images_gen, 7);
    }

    #[test]
    fn format_register_orders_value() {
        assert_eq!(format_register("b", 2), "b: 0x0002");
        assert_eq!(format_register("a", 1), "a: 0x0001");
    }

    #[test]
    fn space_key_resolves_display_keys_without_fabrication() {
        use nerust_core_traits::debugger::{SpaceAccess, SpaceId, SpaceInfo, SpaceTable};
        static ENTRIES: [SpaceInfo; 1] = [SpaceInfo {
            id: SpaceId(0),
            key: "wram",
            name: "WRAM",
            address_bits: 13,
            range: 0x0000..=0x1FFF,
            access: SpaceAccess::ReadWrite,
        }];
        let table = SpaceTable::build(&ENTRIES);
        assert_eq!(
            space_key(
                &table,
                &nerust_core_traits::debugger::DebuggerError::UnknownSpace(SpaceId(9))
            ),
            None,
            "unknown spaces never fabricate a key"
        );
        assert_eq!(
            space_key(
                &table,
                &nerust_core_traits::debugger::DebuggerError::ReadOnlySpace(SpaceId(0))
            ),
            Some("wram")
        );
        assert_eq!(
            space_key(
                &table,
                &nerust_core_traits::debugger::DebuggerError::UnmappedAddress {
                    space: SpaceId(0),
                    addr: 0x2000
                }
            ),
            Some("wram")
        );
        assert_eq!(
            space_key(
                &table,
                &nerust_core_traits::debugger::DebuggerError::BadWidth(0)
            ),
            None
        );
    }

    #[test]
    fn format_panels_covers_all_cell_kinds() {
        use nerust_core_traits::debugger::ColumnKind;
        use nerust_core_traits::debugger::{CellValue, DebugPanel, PanelColumn, PanelRow};
        let panels = [DebugPanel {
            id: "p",
            label_id: "P",
            columns: &[
                PanelColumn {
                    id: "t",
                    label_id: "T",
                    kind: ColumnKind::Text,
                },
                PanelColumn {
                    id: "h",
                    label_id: "H",
                    kind: ColumnKind::Hex { digits: 2 },
                },
                PanelColumn {
                    id: "d",
                    label_id: "D",
                    kind: ColumnKind::Dec,
                },
                PanelColumn {
                    id: "b",
                    label_id: "B",
                    kind: ColumnKind::Bool,
                },
            ],
            rows: vec![PanelRow {
                key: "row",
                cells: vec![
                    CellValue::Text("s".to_string()),
                    CellValue::U64(0xAB),
                    CellValue::U64(241),
                    CellValue::Bool(true),
                ],
            }],
        }];
        assert_eq!(format_panels(&panels), "== P ==\nrow: s AB 241 yes\n");
        assert_eq!(format_panels(&[]), "(no panels)");
        // Kind/cell mismatches degrade instead of panicking.
        assert_eq!(format_cell(&CellValue::U64(7), ColumnKind::Text), "7");
        assert_eq!(
            format_cell(&CellValue::Bool(false), ColumnKind::Hex { digits: 2 }),
            "false"
        );
    }

    #[test]
    fn rgba_image_blits_through_palette() {
        use nerust_core_traits::debugger::{DebugImage, ImageFormat};
        let image = DebugImage {
            id: "left",
            label_id: "Pattern left",
            width: 2,
            height: 1,
            format: ImageFormat::Indexed2bpp,
            palette: vec![[0, 0, 0], [255, 0, 0], [0, 255, 0], [0, 0, 255]],
            pixels: vec![0, 3],
        };
        assert_eq!(
            rgba_image(&image),
            (
                "Pattern left".to_string(),
                2,
                1,
                vec![0, 0, 0, 0xFF, 0, 0, 255, 0xFF]
            )
        );
    }

    #[test]
    fn rgba_image_blits_4bpp_through_palette() {
        use nerust_core_traits::debugger::{DebugImage, ImageFormat};
        let mut palette = vec![[0, 0, 0]; 16];
        palette[15] = [1, 2, 3];
        let image = DebugImage {
            id: "tiles",
            label_id: "Tiles",
            width: 2,
            height: 1,
            format: ImageFormat::Indexed4bpp,
            palette,
            pixels: vec![0, 15],
        };
        assert_eq!(
            rgba_image(&image),
            (
                "Tiles".to_string(),
                2,
                1,
                vec![0, 0, 0, 0xFF, 1, 2, 3, 0xFF]
            )
        );
    }

    #[test]
    fn format_disasm_line_aligns_bytes() {
        let row = DisasmLine {
            addr: 0xC03A,
            bytes: [0x88, 0, 0, 0],
            len: 1,
            text: "DEY".to_string(),
            is_pc: true,
            target: None,
        };
        assert_eq!(format_disasm_line(&row), ">0x0000C03A: 88          DEY");
        let row = DisasmLine {
            addr: 0xC03B,
            bytes: [0xD0, 0xF7, 0, 0],
            len: 2,
            text: "BNE $C034".to_string(),
            is_pc: false,
            target: Some(0xC034),
        };
        assert_eq!(
            format_disasm_line(&row),
            " 0x0000C03B: D0 F7       BNE $C034"
        );
    }

    #[test]
    fn write_transaction_flows_to_commit() {
        use nerust_core_traits::debugger::SpaceId;
        let mut txn = WriteTransaction::Empty;
        assert_eq!(txn.pending(), None);
        assert_eq!(txn.take_commit(), None);
        // Staging without preparation is a no-op.
        txn.stage(0x42);
        assert_eq!(txn.take_commit(), None);
        // Prepare, stage, commit consumes exactly once.
        txn.prepare(SpaceId(0), 0x10, Some(0x00));
        assert_eq!(txn.pending(), None);
        txn.stage(0x42);
        assert_eq!(
            txn.pending(),
            Some(PendingWrite {
                addr: 0x10,
                old: Some(0x00),
                value: 0x42,
            })
        );
        assert_eq!(txn.take_commit(), Some((SpaceId(0), 0x10, 0x42)));
        assert_eq!(txn, WriteTransaction::Empty);
        assert_eq!(txn.take_commit(), None);
    }

    #[test]
    fn write_transaction_prepare_overwrites() {
        use nerust_core_traits::debugger::SpaceId;
        let mut txn = WriteTransaction::Empty;
        txn.prepare(SpaceId(0), 0x10, Some(0x00));
        txn.stage(0x42);
        // Fresh selection drops the staged value.
        txn.prepare(SpaceId(0), 0x20, Some(0xFF));
        assert_eq!(txn.pending(), None);
        assert_eq!(txn.take_commit(), None);
    }

    #[test]
    fn write_transaction_traffic_kills_uncommitted() {
        use nerust_core_traits::debugger::SpaceId;
        let mut txn = WriteTransaction::Empty;
        // Empty traffic is a no-op.
        txn.on_state_changing_traffic();
        assert_eq!(txn, WriteTransaction::Empty);
        // Prepared and staged values die on state-changing traffic.
        txn.prepare(SpaceId(0), 0x10, Some(0x00));
        txn.on_state_changing_traffic();
        assert_eq!(txn, WriteTransaction::Empty);
        txn.prepare(SpaceId(0), 0x10, Some(0x00));
        txn.stage(0x42);
        txn.on_state_changing_traffic();
        assert_eq!(txn, WriteTransaction::Empty);
        assert_eq!(txn.pending(), None);
        // Explicit cancel clears too.
        txn.prepare(SpaceId(0), 0x10, Some(0x00));
        txn.clear();
        assert_eq!(txn, WriteTransaction::Empty);
    }
}
