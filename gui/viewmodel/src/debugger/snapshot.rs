//! One drain's worth of interpreted debugger data. The session
//! fills it from a single batched core pass; frontends render
//! from it and never call the core themselves.

use nerust_core_traits::debugger::{DebugPanel, DisasmLine, HexRow};

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
