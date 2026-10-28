//! Framework debugger vocabulary: gestures, scopes, chrome, rows, and
//! opaque session handles. View definitions own bridges, models, and
//! drivers; native frontends hold [`DebugHandle`]s and render snapshots.

/// Opaque debugger session: one bridge plus its model and drain driver.
/// Created by the app (`open_debug`) and never dereferenced natively.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DebugHandle(pub u64);

/// Native adapters translate gestures into these intents, never emulator calls.
#[derive(Debug, Clone)]
pub enum DebuggerEvent {
    RefreshPressed,
    SpacePrev,
    SpaceNext,
    TogglePausePressed,
    StepFramePressed,
    StepInstrPressed,
    PpuOpenPressed,
    MemInputChanged(String),
    MemGo,
    MemPage(i32),
    DisInputChanged(String),
    DisGo,
    FollowPcToggle,
    DisLineClicked(u32),
    FollowTarget(u32),
    DisBack,
    BookmarkInputChanged(String),
    BookmarkAdd,
    BookmarkJump(u32),
    BookmarkDelete(u32),
    WatchSet,
    WatchClear,
    FreezeInputChanged(String),
    FreezeSet,
    EditFreezeSet,
    FreezeClear,
    RowSelected(u32),
    EditInputChanged(String),
    EditWrite,
    ConfirmWrite,
    CancelWrite,
    RegSelected(&'static str),
    RegInputChanged(String),
    RegSet,
}

/// Minimum view sections affected by an intent (header accompanies every scope).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DebuggerScope {
    All,
    Dump,
    Disasm,
    State,
    Edit,
    Bookmarks,
    Header,
}

/// Owned native effects of one gesture batch. Apply these after releasing
/// controller borrows; highlight-only gestures skip list rebuilds.
#[derive(Debug, Default)]
pub struct DebuggerEffects {
    pub scope: Option<DebuggerScope>,
    pub open_ppu: bool,
    /// Clear native staged inputs; the gesture belonged to a replaced model.
    pub epoch_reset: bool,
    /// `poll` requests another scheduled poll. Dispatch only queues work;
    /// call `poll` after dispatch even when this flag is false.
    pub pending: bool,
}

/// Live-tick outcome: pause chrome may have moved, nothing else rebuilds.
#[derive(Debug, Default)]
pub struct DebuggerLiveEffects {
    pub changed: bool,
    pub running: bool,
    pub pending: bool,
}

/// Pre-localized debugger chrome. Frontends render it verbatim.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash)]
pub struct DebuggerChrome {
    pub debugger: String,
    pub pause: String,
    pub resume: String,
    pub step_frame: String,
    pub step_instr: String,
    pub ppu_viewer: String,
    pub refresh: String,
    pub space: String,
    pub prev: String,
    pub next: String,
    pub page: String,
    pub addr_hex: String,
    pub go: String,
    pub back: String,
    pub disassembly: String,
    pub follow_pc: String,
    pub follow_pc_tag: String,
    pub fixed_tag: String,
    pub no_disassembly: String,
    pub disassembly_hint: String,
    pub registers: String,
    pub pick_register: String,
    pub hex: String,
    pub set: String,
    pub select_dump_row: String,
    pub write: String,
    pub hex_byte: String,
    pub watch: String,
    pub freeze: String,
    pub confirm: String,
    pub cancel: String,
    pub watch_none: String,
    pub freeze_none: String,
    pub clear: String,
    pub bookmarks: String,
    pub label: String,
    pub add: String,
    pub memory: String,
}

impl DebuggerChrome {
    /// Toolbar pause toggle: resume when paused, pause otherwise.
    pub fn pause_label(&self, paused: bool) -> &str {
        if paused { &self.resume } else { &self.pause }
    }

    /// Back button with history depth, e.g. `Back (3)`.
    pub fn back_label(&self, back_len: usize) -> String {
        format!("{} ({back_len})", self.back)
    }

    /// Disassembly title: follow tag when tracking PC, anchored address otherwise.
    pub fn disassembly_title(&self, follow_pc: bool, anchor: u32) -> String {
        if follow_pc {
            format!("{} {}", self.disassembly, self.follow_pc_tag)
        } else {
            format!("{} @0x{anchor:08X} {}", self.disassembly, self.fixed_tag)
        }
    }

    /// Register target line: `Set <name>` once picked.
    pub fn set_target_label(&self, name: Option<&str>) -> String {
        match name {
            Some(name) => format!("{} {name}", self.set),
            None => self.pick_register.clone(),
        }
    }

    /// Write button carries its target address once a row is selected.
    pub fn write_target_label(&self, selected: Option<(u32, Option<u8>)>) -> String {
        match selected {
            Some((addr, _)) => format!("{} 0x{addr:08X}", self.write),
            None => self.write.clone(),
        }
    }
}

/// Pre-localized PPU viewer chrome for the viewer opened from a session.
#[derive(Debug, Clone, Default)]
pub struct PpuChrome {
    pub title: String,
    pub hover_empty: String,
    pub no_images: String,
    pub panels_title: String,
}

/// One disassembly row: pre-formatted line plus navigation addresses.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct DisasmRow {
    pub addr: u32,
    pub is_pc: bool,
    pub text: String,
    pub target: Option<u32>,
}

/// One hex-dump row: address plus owned bytes.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct DumpRow {
    pub addr: u32,
    pub bytes: Vec<u8>,
}

/// One register row: pre-formatted text plus write-target name.
/// Names are static register identifiers; only the text is localized.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RegRow {
    pub name: &'static str,
    pub text: String,
}

/// Formatted debugger rows. Frontends render strings and route addresses
/// back as gestures; no formatter or row-type import needed.
#[derive(Debug, Clone, Default)]
pub struct DebuggerRows {
    pub disasm: Vec<DisasmRow>,
    pub dump: Vec<DumpRow>,
    pub changed_addrs: Vec<u32>,
    pub regs: Vec<RegRow>,
    pub status_text: String,
    pub paused: bool,
    pub pending_text: Option<String>,
    /// Watch line: live value when watched, placeholder otherwise.
    pub watch_text: String,
    /// Freeze line: frozen byte when set, placeholder otherwise.
    pub freeze_text: String,
    /// Dump selection line, including the old byte when readable.
    pub selected_line: String,
}

/// One 2x-scaled PPU picture with its display size.
#[derive(Debug, Clone)]
pub struct ScaledPicture {
    pub label: String,
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

/// Owned navigation and input snapshot: every text field a native rebuild
/// reads, plus the anchor inputs for follow/disassembly placement.
#[derive(Debug, Clone, Default)]
pub struct DebugInputs {
    pub mem_input: String,
    pub dis_input: String,
    pub follow_pc: bool,
    pub back_len: usize,
    pub dis_addr: Option<u32>,
    pub mem_addr: Option<u32>,
    pub bm_input: String,
    pub bookmarks: Vec<(u32, String)>,
    pub freeze_input: String,
    pub edit_input: String,
    pub reg_input: String,
    pub watch: Option<u32>,
    pub watch_value: Option<u8>,
    pub freeze: Option<(u32, u8)>,
}

/// One coherent debugger snapshot: chrome, formatted rows, inputs, and the
/// selection natives need for highlight and write-target rendering.
#[derive(Debug, Clone)]
pub struct DebugSnapshot {
    pub chrome: DebuggerChrome,
    pub rows: DebuggerRows,
    pub inputs: DebugInputs,
    pub selected: Option<(u32, Option<u8>)>,
    pub reg_target: Option<&'static str>,
    pub space_name: String,
    pub has_spaces: bool,
}

/// Pin-column marker without naming the formatter crate.
/// Mirrors the canonical rule: a pinned PC row keeps blanks because the
/// PC `>` already ships inside [`DisasmRow::text`].
pub fn pin_marker(is_pc: bool, is_pinned: bool) -> &'static str {
    if !is_pc && is_pinned { "@ " } else { "  " }
}
