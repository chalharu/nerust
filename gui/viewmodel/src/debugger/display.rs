//! One rebuild's worth of debugger display data, written by the
//! drain and read by frontends.

use nerust_core_traits::debugger::{DebugPanel, DisasmLine, HexRow};

use super::{PendingWrite, StatusMessage};

/// One rebuild's worth of display data. Written by the drain, read
/// by view code; widgets never lock session state mid-build. Rows
/// travel structured; text formatting is presentation's job.
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

    /// Restore the last paused sections over a running drain, so Resume
    /// keeps inspectable state instead of blanking. Fresh images,
    /// status, and pause chrome stay untouched.
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
