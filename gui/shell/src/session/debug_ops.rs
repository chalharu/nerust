//! Debugger views: batched, paused-gated reads plus the shell-owned write transaction.
//! Frontends render from snapshots and never touch the core.

use std::sync::Arc;

use nerust_gui_viewmodel::debugger as debug_vm;

use super::{SessionHandle, osd::TickMetrics};
use crate::debug::{CoreFailure, ImageNeed, TrafficKind};

impl SessionHandle {
    /// Run one action batch and return the render-ready display.
    /// Tests/headless only; frontends use the flight.
    pub fn debug_drain(
        &self,
        actions: &[crate::debug::DebugAction],
        input: &crate::debug::DebugDrainInput,
    ) -> crate::debug::DebugDrainOutput {
        let mut flight = self.debug_drain_begin(actions, input);
        loop {
            match flight.poll() {
                crate::debug::DrainPoll::Done(output) => return *output,
                crate::debug::DrainPoll::Pending => {
                    std::thread::sleep(std::time::Duration::from_millis(1));
                }
            }
        }
    }

    /// Start one action batch without blocking; safe on the UI thread.
    /// Poll to `Done`; a newer batch drops the old flight, newest view wins.
    pub fn debug_drain_begin(
        &self,
        actions: &[crate::debug::DebugAction],
        input: &crate::debug::DebugDrainInput,
    ) -> crate::debug::DrainFlight {
        self.debug_drain_begin_with_deadline(actions, input, crate::debug::DEBUG_DRAIN_TIMEOUT)
    }

    /// [`SessionHandle::debug_drain_begin`] with an explicit per-await deadline.
    /// Tests inject milliseconds to cover the timeout fallbacks without waiting.
    pub(crate) fn debug_drain_begin_with_deadline(
        &self,
        actions: &[crate::debug::DebugAction],
        input: &crate::debug::DebugDrainInput,
        deadline: std::time::Duration,
    ) -> crate::debug::DrainFlight {
        crate::debug::DrainFlight::begin(self.drain_ctx(), actions, input, deadline)
    }

    /// Start one images-only live fetch without blocking; drive the flight with `poll` per tick.
    /// Cache hits and coreless sessions resolve on the first poll.
    pub fn debug_live_images_begin(&self, need_images: bool) -> crate::debug::LiveImagesFlight {
        crate::debug::LiveImagesFlight::begin(
            self.drain_ctx().core,
            ImageNeed::from_flag(need_images),
            crate::debug::DEBUG_DRAIN_TIMEOUT,
        )
    }

    /// Scoped transaction access: the guard never escapes, so holders cannot deadlock.
    /// Exactly one logging point for the poison recovery.
    fn with_txn<R>(&self, f: impl FnOnce(&mut debug_vm::WriteTransaction) -> R) -> R {
        match self.debug_txn.lock() {
            Ok(mut txn) => f(&mut txn),
            Err(error) => {
                log::error!("debug transaction mutex poisoned, recovering");
                f(&mut error.into_inner())
            }
        }
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

    /// Owned context for one async drain flight: a boxed live core plus
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
        use crate::debug::{assemble_snapshot_parts, fold_watch_byte};
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
        let Some(parts) = assemble_snapshot_parts(batch) else {
            return snapshot;
        };
        snapshot.available = true;
        snapshot.regs = parts.regs;
        snapshot.reg_names = parts.reg_names;
        snapshot.dump_rows = parts.dump_rows;
        snapshot.panels = parts.panels;
        // Anchor without name-scanning: explicit pin wins, otherwise
        // the batched program counter (follow mode only).
        snapshot.pc = parts.anchor;
        snapshot.disasm_lines = parts.disasm;
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
        snapshot.watch = watch.and_then(|addr| fold_watch_byte(&snapshot.dump_rows, addr));
        snapshot
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
        self.with_txn(|txn| txn.prepare(space, addr, old));
        old
    }

    /// Stage the parsed value; only a prepared transaction accepts it.
    pub fn debug_stage_write(&self, value: u64) {
        self.with_txn(|txn| txn.stage(value));
    }

    /// Confirm-row data, if fully staged. The frontend renders the
    /// wording; the session only reports the staged values.
    pub fn debug_pending_text(&self) -> Option<debug_vm::PendingWrite> {
        self.with_txn(|txn| txn.pending())
    }

    /// Commit the staged write, consuming the transaction.
    pub fn debug_commit_write(&self) -> debug_vm::StatusMessage {
        use nerust_core_traits::debugger::MemoryWrite;
        let staged = self.with_txn(|txn| txn.take_commit());
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
        self.with_txn(|txn| txn.clear());
    }

    /// Report drain traffic: kills staged-but-uncommitted values from previous drains.
    /// Select/stage/commit traffic passes [`TrafficKind::Transactional`], consumed by this drain.
    pub fn debug_note_traffic(&self, traffic: TrafficKind) {
        if traffic == TrafficKind::StateChanging {
            self.with_txn(|txn| txn.on_state_changing_traffic());
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

#[cfg(test)]
mod drain_ctx_cov_tests {
    use crate::session::test_util::test_session;

    #[test]
    fn drain_ctx_exposes_narrow_debug_port() {
        let session = test_session();
        let ctx = session.drain_ctx();
        assert!(ctx.core_port().is_some());
    }

    #[test]
    fn drain_ctx_recovers_and_images_skip_resolves() {
        use crate::debug::{DEBUG_DRAIN_TIMEOUT, ImageNeed, LiveImagesFlight, LiveImagesPoll};
        let session = test_session();
        let ctx = session.drain_ctx();
        let _pending = ctx.with_txn(|txn| txn.pending());
        let mut flight = LiveImagesFlight::begin(None, ImageNeed::Skip, DEBUG_DRAIN_TIMEOUT);
        assert!(matches!(flight.poll(), LiveImagesPoll::Done(None)));
        let mut flight = LiveImagesFlight::begin(
            ctx.core.as_ref().map(|core| core.drain_handle()),
            ImageNeed::Skip,
            DEBUG_DRAIN_TIMEOUT,
        );
        assert!(matches!(flight.poll(), LiveImagesPoll::Done(_)));
    }
}

#[cfg(test)]
mod debug_ops_cov_tests {
    use super::*;
    use crate::debug::ImageNeed;
    use crate::session::test_util::test_session;
    use nerust_core_traits::debugger::SpaceId;

    #[test]
    fn coreless_debug_paths_report_no_core() {
        use std::sync::Arc;
        let caps = test_session().capabilities;
        let registry = Arc::new(crate::registry::SystemRegistry::new(vec![]));
        let audio = Arc::new(nerust_core_traits::audio::AudioBackendRegistry::new());
        let session = super::SessionHandle::new_ephemeral(caps, registry, audio);
        assert!(session.debug_spaces().is_empty());
        assert!(session.debug_snapshot_ready());
        assert!(session.debug_snapshot_ready_tick(&TickMetrics::default()));
        assert!(
            !session
                .debug_snapshot(None, 0, None, ImageNeed::Skip, None)
                .available
        );
        assert!(crate::session::test_util::poll_live_images(&session, ImageNeed::Skip).is_none());
        assert_eq!(session.debug_read_byte(SpaceId(0), 0), None);
        assert!(!session.debug_write_byte(SpaceId(0), 0, 1));
        assert!(matches!(
            session.debug_write_register("a", 1),
            debug_vm::StatusMessage::NoCore
        ));
        assert_eq!(session.debug_pending_text(), None);
        assert!(matches!(
            session.debug_commit_write(),
            debug_vm::StatusMessage::NothingToWrite
        ));
        session.debug_clear_write();
        session.debug_note_traffic(TrafficKind::StateChanging);
        session.debug_note_traffic(TrafficKind::Transactional);
        assert!(matches!(
            session.debug_pause(),
            debug_vm::StatusMessage::NoCore
        ));
        assert!(matches!(
            session.debug_resume(),
            debug_vm::StatusMessage::NoCore
        ));
    }

    #[test]
    fn loaded_debug_paths_answer() {
        use nerust_core_traits::factory::load::MediaObject;
        let temp_dir = crate::test_helpers::unique_temp_dir("debug-ops-loaded");
        let mut session = test_session();
        let factory = session.active_factory_cloned().unwrap();
        let options = factory.default_load_options();
        let view = crate::session::test_util::test_view(&session);
        let resolved = factory.resolve_load_request(&view, options).unwrap();
        session
            .load_resolved(
                MediaObject::new(
                    Some(temp_dir.join("test.nes")),
                    crate::test_helpers::test_rom(),
                ),
                resolved,
            )
            .unwrap();
        assert_eq!(session.debug_spaces().len(), 1);
        assert!(session.debug_read_byte(SpaceId(0), 0).is_some());
        assert!(session.debug_write_byte(SpaceId(0), 0x10, 0xAB));
        assert_eq!(session.debug_read_byte(SpaceId(0), 0x10), Some(0xAB));
        let staged = session.debug_prepare_write(SpaceId(0), 0x10);
        assert_eq!(staged, Some(0xAB));
        session.debug_stage_write(0xCD);
        assert!(session.debug_pending_text().is_some());
        session.debug_clear_write();
        assert_eq!(session.debug_pending_text(), None);
        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn debug_write_register_round_trips_and_refuses() {
        use nerust_core_traits::factory::load::MediaObject;
        use nerust_gui_viewmodel::debugger::StatusMessage as S;
        let temp_dir = crate::test_helpers::unique_temp_dir("debug-ops-reg");
        let mut session = test_session();
        // ROM-less core has no debugger: refusal without transport noise.
        assert!(matches!(
            session.debug_write_register("a", 5),
            S::RegisterRefused(_)
        ));
        let factory = session.active_factory_cloned().unwrap();
        let options = factory.default_load_options();
        let view = crate::session::test_util::test_view(&session);
        let resolved = factory.resolve_load_request(&view, options).unwrap();
        session
            .load_resolved(
                MediaObject::new(
                    Some(temp_dir.join("test.nes")),
                    crate::test_helpers::test_rom(),
                ),
                resolved,
            )
            .unwrap();
        assert!(matches!(
            session.debug_write_register("a", 5),
            S::RegisterWrote {
                name: "a",
                value: 5
            }
        ));
        assert!(matches!(
            session.debug_write_register("bogus", 0),
            S::RegisterRefused(_)
        ));
        assert!(matches!(session.debug_pause(), S::Paused));
        assert!(matches!(session.debug_resume(), S::Resume));
        let _ = std::fs::remove_dir_all(&temp_dir);
    }
}
