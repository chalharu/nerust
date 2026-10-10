//! Shared debugger action drain: one phase order for every frontend.
//!
//! Both frontends map widgets to [`DebugAction`]; [`SessionHandle::debug_drain`]
//! owns the load-bearing phase order (execution, cancel/commit,
//! stale-kill, prepare/stage, freeze, snapshot). Keeping this in one
//! place keeps tao and GTK behavior identical by construction and
//! makes the order unit-testable without a GUI.

use nerust_core_traits::debugger::{SpaceId, StepUnit};
use nerust_gui_viewmodel::debugger::{self as vm, DisplayCache};

use crate::session::SessionHandle;

/// One debugger action batch. Same variants as the former per-frontend
/// request enums; frontends only translate widgets into these.
#[derive(Debug, Clone, Copy)]
pub enum DebugAction {
    Refresh,
    Pause,
    Resume,
    TogglePause,
    StepFrame,
    StepInstr,
    MemNav,
    SelectRow(u32),
    /// Stage a parsed value into the shell-owned transaction.
    EditStage(u64),
    EditCancel,
    WriteConfirm,
}

/// Frontend-owned state the drain reads but never stores.
pub struct DebugDrainInput<'a> {
    /// Space for byte reads, selection prepare, and freeze re-apply.
    pub space: Option<SpaceId>,
    /// Dump page anchor.
    pub mem_addr: u32,
    /// Disassembly anchor (`None` follows the program counter).
    pub dis_addr: Option<u32>,
    /// Watched address for the read-only live value.
    pub watch: Option<u32>,
    /// Frozen `(address, value)` re-applied every batch.
    pub freeze: Option<(u32, u8)>,
    /// Previous dump rows for `*` diff marks.
    pub prev_dump_rows: &'a [(u32, String)],
    /// Fetch system images (pattern tables). False when no PPU
    /// viewer is open: CHR decode is skipped, not just unused.
    pub need_images: bool,
}

/// Everything a frontend needs to render after a batch.
pub struct DebugDrainOutput {
    /// Ready-to-render display (diff, watch value, pending text,
    /// and final status included).
    pub display: DisplayCache,
    /// Fresh `(address, old byte)` when the batch selected a row
    /// (`None` means the selection is unchanged).
    pub reselected: Option<(u32, Option<u8>)>,
}

impl SessionHandle {
    /// Run one action batch and return the render-ready display.
    /// Phase order is load-bearing: execution first, then
    /// cancel/commit, then the stale-kill (before prepare/stage),
    /// then selection prepare and staging, freeze re-apply, and a
    /// single batched snapshot last.
    pub fn debug_drain(
        &self,
        actions: &[DebugAction],
        input: &DebugDrainInput,
    ) -> DebugDrainOutput {
        let mut status = String::new();
        let mut state_changing = false;
        for action in actions {
            let outcome = match action {
                DebugAction::Refresh | DebugAction::MemNav | DebugAction::SelectRow(_) => {
                    state_changing = true;
                    continue;
                }
                DebugAction::Pause => self.debug_pause(),
                DebugAction::Resume => self.debug_resume(),
                DebugAction::TogglePause => {
                    if self.debug_paused() {
                        self.debug_resume()
                    } else {
                        self.debug_pause()
                    }
                }
                DebugAction::StepFrame => {
                    state_changing = true;
                    self.debug_step(StepUnit::Frame)
                }
                DebugAction::StepInstr => {
                    state_changing = true;
                    self.debug_step(StepUnit::Instruction)
                }
                // Transaction traffic never kills: it builds the
                // transaction this batch consumes.
                DebugAction::EditStage(_) | DebugAction::EditCancel | DebugAction::WriteConfirm => {
                    continue;
                }
            };
            status = outcome;
        }
        // Cancel/commit first: cancel clears, commit writes and consumes.
        for action in actions {
            match action {
                DebugAction::EditCancel => self.debug_clear_write(),
                DebugAction::WriteConfirm => {
                    status = self.debug_commit_write();
                }
                _ => {}
            }
        }
        // Kill staged-but-uncommitted values from previous batches first;
        // SelectRow prepares fresh state after this.
        if state_changing {
            self.debug_note_traffic(true);
        }
        // Row selection reads the old byte for the selection display.
        let mut reselected = None;
        for action in actions {
            if let DebugAction::SelectRow(addr) = action {
                let old = input
                    .space
                    .and_then(|space| self.debug_read_byte(space, *addr));
                reselected = Some((*addr, old));
                if let Some(space) = input.space {
                    self.debug_prepare_write(space, *addr);
                }
            }
            if let DebugAction::EditStage(value) = action {
                self.debug_stage_write(*value);
            }
        }
        // Frozen values re-apply on every batch (silent by design).
        if let Some((addr, value)) = input.freeze
            && let Some(space) = input.space
        {
            let _ = self.debug_write_byte(space, addr, value);
        }
        let snapshot = self.debug_snapshot(
            input.space,
            input.mem_addr,
            input.dis_addr,
            input.need_images,
        );
        let watch_value = input
            .watch
            .and_then(|addr| input.space.and_then(|s| self.debug_read_byte(s, addr)));
        let pending_text = self.debug_pending_text();
        let status = if status.is_empty() {
            if self.debug_paused() {
                "paused".to_string()
            } else {
                "running".to_string()
            }
        } else {
            status
        };
        let display = DisplayCache {
            regs: snapshot.regs,
            diff: vm::diff_rows(input.prev_dump_rows, &snapshot.dump_rows),
            dump_rows: snapshot.dump_rows,
            disasm_lines: snapshot.disasm_lines,
            panels: snapshot.panels,
            images: snapshot.images,
            watch_value,
            status,
            pending_text,
        };
        DebugDrainOutput {
            display,
            reselected,
        }
    }
}
