//! Debugger views: batched, paused-gated reads plus the shell-owned write transaction.
//! Frontends render from snapshots and never touch the core.

use std::sync::{Arc, MutexGuard};

use nerust_gui_viewmodel::debugger as debug_vm;

#[cfg(test)]
use super::LiveImages;
use super::{SessionHandle, osd::TickMetrics};
use crate::debug::{CoreFailure, ImageNeed, TrafficKind};

impl SessionHandle {
    /// Poison-recovering transaction lock: plain data survives a panicked holder.
    /// Exactly one logging point for the recovery.
    fn lock_txn(&self) -> MutexGuard<'_, debug_vm::WriteTransaction> {
        self.debug_txn.lock().unwrap_or_else(|error| {
            log::error!("debug transaction mutex poisoned, recovering");
            error.into_inner()
        })
    }

    pub fn debug_spaces(&self) -> Vec<nerust_core_traits::debugger::SpaceInfo> {
        self.emu
            .core()
            .ok()
            .and_then(|core| core.memory_spaces().ok())
            .unwrap_or_default()
    }

    /// Whether a debugger snapshot read is safe: true while paused, and with no core.
    /// Deliberately different from `paused`, which reports the emulation pause flag.
    pub fn debug_snapshot_ready(&self) -> bool {
        self.debug_snapshot_ready_tick(&self.tick_metrics())
    }

    /// Tick-threaded form of [`debug_snapshot_ready`](Self::debug_snapshot_ready):
    /// no metrics lock here, so per-tick debugger gates share the pass tick.
    pub fn debug_snapshot_ready_tick(&self, tick: &TickMetrics) -> bool {
        self.emu.core().ok().is_none_or(|_| tick.paused)
    }

    /// Owned context for one async drain flight: a cloned core plus
    /// the shared write transaction. See [`DrainCtx`].
    pub(crate) fn drain_ctx(&self) -> crate::debug::DrainCtx {
        crate::debug::DrainCtx::new(
            self.emu.core().ok().map(|core| core.drain_handle()),
            Arc::clone(&self.debug_txn),
        )
    }

    /// Single batched snapshot for one drain: one inspect, one disassembly, one images pass.
    /// Unavailable sections carry degenerates; rows travel structured for frontends to format.
    pub fn debug_snapshot(
        &self,
        space: Option<nerust_core_traits::debugger::SpaceId>,
        mem_addr: u32,
        dis_addr: Option<u32>,
        need_images: ImageNeed,
        watch: Option<u32>,
    ) -> debug_vm::DebugSnapshot {
        use nerust_core_traits::debugger::{
            DEBUG_DISASM_ROWS, DEBUG_DUMP_PAGE_ROWS, InspectRequest, SnapshotRequest,
        };
        let mut snapshot = debug_vm::DebugSnapshot::default();
        let core = match self.emu.core().ok() {
            Some(core) => core,
            None => return snapshot,
        };
        // One thread crossing for inspect, anchor, and disassembly.
        let batch = match core.debug_snapshot_batch(SnapshotRequest {
            inspect: InspectRequest {
                space,
                addr: space.map(|_| mem_addr),
                rows: DEBUG_DUMP_PAGE_ROWS,
            },
            anchor: dis_addr,
            disasm_rows: DEBUG_DISASM_ROWS,
        }) {
            Ok(Ok(batch)) => batch,
            Ok(Err(_)) | Err(_) => return snapshot,
        };
        let inspected = match batch.inspect {
            Ok(inspected) => inspected,
            Err(_) => return snapshot,
        };
        snapshot.available = true;
        snapshot.regs = inspected
            .registers
            .iter()
            .map(|&(name, value)| (name, value))
            .collect();
        snapshot.reg_names = inspected.registers.iter().map(|(name, _)| *name).collect();
        snapshot.dump_rows = inspected.dump.rows.iter().copied().collect();
        snapshot.panels = inspected.panels.iter().cloned().collect();
        // Anchor without name-scanning: explicit pin wins, otherwise
        // the batched program counter (follow mode only).
        let anchor = batch.anchor;
        snapshot.pc = anchor;
        snapshot.disasm_lines = batch.disasm;
        let (images_gen, images) = if need_images == ImageNeed::Fetch {
            match core.debug_images() {
                Ok(Ok((generation, images))) => (generation, images),
                _ => (0, Vec::new()),
            }
        } else {
            (0, Vec::new())
        };
        snapshot.images_gen = images_gen;
        snapshot.images = images.iter().map(debug_vm::rgba_image).collect();
        // Watch folded into the page read when it lands on it;
        // off-page watches still fall back to a single-byte inspect.
        snapshot.watch = watch.and_then(|addr| {
            inspected
                .dump
                .rows
                .iter()
                .find(|row| addr >= row.addr && addr < row.addr + u32::from(row.valid))
                .map(|row| row.bytes[(addr - row.addr) as usize])
        });
        snapshot
    }

    /// Images-only refresh for the live PPU follow while the core runs.
    /// Blocking wrapper for tests only; live ticks use the flight.
    #[cfg(test)]
    pub fn debug_live_images(&self, need_images: ImageNeed) -> LiveImages {
        let mut flight = self.debug_live_images_begin(matches!(need_images, ImageNeed::Fetch));
        loop {
            match flight.poll() {
                crate::debug::LiveImagesPoll::Done(live) => return live,
                crate::debug::LiveImagesPoll::Pending => {
                    std::thread::sleep(std::time::Duration::from_millis(1));
                }
            }
        }
    }

    /// Single-byte read for old-value display and watch values.
    pub fn debug_read_byte(
        &self,
        space: nerust_core_traits::debugger::SpaceId,
        addr: u32,
    ) -> Option<u8> {
        let core = self.emu.core().ok()?;
        let result = core
            .inspect(nerust_core_traits::debugger::InspectRequest {
                space: Some(space),
                addr: Some(addr),
                rows: 1,
            })
            .ok()?
            .ok()?;
        let row = result.dump.rows.first()?;
        (row.valid > 0).then_some(row.bytes[0])
    }

    /// Single-byte write for freeze re-application. Silent by design:
    /// the rows are the evidence. `false` when unwritable.
    pub fn debug_write_byte(
        &self,
        space: nerust_core_traits::debugger::SpaceId,
        addr: u32,
        value: u8,
    ) -> bool {
        use nerust_core_traits::debugger::MemoryWrite;
        let core = match self.emu.core().ok() {
            Some(core) => core,
            None => return false,
        };
        matches!(
            core.write_memory(MemoryWrite {
                space,
                addr,
                width: 1,
                value: u64::from(value),
            }),
            Ok(Ok(()))
        )
    }

    /// Single-register write for the register viewer.
    pub fn debug_write_register(&self, name: &'static str, value: u64) -> debug_vm::StatusMessage {
        let core = match self.emu.core().ok() {
            Some(core) => core,
            None => return CoreFailure::no_core().thread_status(),
        };
        match core.write_register(name, value) {
            Ok(Ok(())) => debug_vm::StatusMessage::RegisterWrote { name, value },
            Ok(Err(error)) => CoreFailure::from(error).register_status(),
            Err(error) => CoreFailure::from(error).thread_status(),
        }
    }

    /// Select a row: read the old byte and prepare the transaction,
    /// overwriting any state.
    pub fn debug_prepare_write(
        &self,
        space: nerust_core_traits::debugger::SpaceId,
        addr: u32,
    ) -> Option<u8> {
        let old = self.debug_read_byte(space, addr);
        self.lock_txn().prepare(space, addr, old);
        old
    }

    /// Stage the parsed value; only a prepared transaction accepts it.
    pub fn debug_stage_write(&self, value: u64) {
        self.lock_txn().stage(value);
    }

    /// Confirm-row data, if fully staged. The frontend renders the
    /// wording; the session only reports the staged values.
    pub fn debug_pending_text(&self) -> Option<debug_vm::PendingWrite> {
        self.lock_txn().pending()
    }

    /// Commit the staged write, consuming the transaction.
    pub fn debug_commit_write(&self) -> debug_vm::StatusMessage {
        use nerust_core_traits::debugger::MemoryWrite;
        let staged = self.lock_txn().take_commit();
        let (space, addr, value) = match staged {
            Some(staged) => staged,
            None => return debug_vm::StatusMessage::NothingToWrite,
        };
        let core = match self.emu.core().ok() {
            Some(core) => core,
            None => return CoreFailure::no_core().thread_status(),
        };
        match core.write_memory(MemoryWrite {
            space,
            addr,
            width: 1,
            value,
        }) {
            Ok(Ok(())) => debug_vm::StatusMessage::WroteMemory { addr, value },
            Ok(Err(error)) => CoreFailure::from(error).write_status(),
            Err(error) => CoreFailure::from(error).thread_status(),
        }
    }

    /// Drop any uncommitted transaction (cancel, space switch).
    pub fn debug_clear_write(&self) {
        self.lock_txn().clear();
    }

    /// Report drain traffic: kills staged-but-uncommitted values from previous drains.
    /// Select/stage/commit traffic passes [`TrafficKind::Transactional`], consumed by this drain.
    pub fn debug_note_traffic(&self, traffic: TrafficKind) {
        if traffic == TrafficKind::StateChanging {
            self.lock_txn().on_state_changing_traffic();
        }
    }

    /// Pause for debugger control.
    pub fn debug_pause(&self) -> debug_vm::StatusMessage {
        match self.emu.core().ok() {
            Some(core) => match core.pause() {
                Ok(()) => debug_vm::StatusMessage::Paused,
                Err(error) => CoreFailure::from(error).pause_status(true),
            },
            None => CoreFailure::no_core().thread_status(),
        }
    }

    /// Resume for debugger control.
    pub fn debug_resume(&self) -> debug_vm::StatusMessage {
        match self.emu.core().ok() {
            Some(core) => match core.resume() {
                Ok(()) => debug_vm::StatusMessage::Resume,
                Err(error) => CoreFailure::from(error).pause_status(false),
            },
            None => CoreFailure::no_core().thread_status(),
        }
    }

    /// Advance one unit for debugger control. Returns the stepped
    /// count or a structured failure.
    pub fn debug_step(
        &self,
        unit: nerust_core_traits::debugger::StepUnit,
    ) -> debug_vm::StatusMessage {
        let core = match self.emu.core().ok() {
            Some(core) => core,
            None => return CoreFailure::no_core().thread_status(),
        };
        match core.step(unit) {
            Ok(Ok(count)) => debug_vm::StatusMessage::Stepped(count),
            Ok(Err(error)) => CoreFailure::from(error).step_status(),
            Err(error) => CoreFailure::from(error).thread_status(),
        }
    }
}
