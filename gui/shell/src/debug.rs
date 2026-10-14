//! Shared debugger action drain: one phase order for every frontend.
//! Blocking wrapper spins flights; frontends use begin-plus-poll.

use std::{
    collections::VecDeque,
    sync::{Arc, Mutex, mpsc::Receiver},
    time::{Duration, Instant},
};

use nerust_core_traits::debugger::{
    DEBUG_DISASM_ROWS, DEBUG_DUMP_PAGE_ROWS, DebugImage, DebuggerError, HexRow, InspectError,
    InspectRequest, InspectResult, SnapshotResult, SpaceId, StepUnit,
};
use nerust_gui_viewmodel::debugger::{self as vm, DisplayCache};

use crate::{emu_core::EmuCore, session::SessionHandle};

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
    /// Write a parsed value into the selected register
    /// ([`DebugDrainInput::reg_target`]).
    RegisterWrite(u64),
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
    /// Selected register for [`DebugAction::RegisterWrite`].
    pub reg_target: Option<&'static str>,
    /// Frozen `(address, value)` re-applied every batch.
    pub freeze: Option<(u32, u8)>,
    /// Previous dump rows for `*` diff marks.
    pub prev_dump_rows: &'a [HexRow],
    /// Fetch system images (pattern tables). False when no PPU
    /// viewer is open: CHR decode is skipped, not just unused.
    pub need_images: bool,
}

/// Everything a frontend needs to render after a batch.
pub struct DebugDrainOutput {
    /// Render-ready display (diff, watch value, pending write, and
    /// structured status included; the frontend renders wording).
    pub display: DisplayCache,
    /// Fresh `(address, old byte)` when the batch selected a row
    /// (`None` means the selection is unchanged).
    pub reselected: Option<(u32, Option<u8>)>,
}

impl SessionHandle {
    /// Run one action batch and return the render-ready display.
    /// Phase order: execution, cancel/commit, stale-kill, prepare and
    /// staging, freeze re-apply, single batched snapshot last.
    /// Tests/headless only; frontends use the flight.
    pub fn debug_drain(
        &self,
        actions: &[DebugAction],
        input: &DebugDrainInput,
    ) -> DebugDrainOutput {
        let mut flight = self.debug_drain_begin(actions, input);
        loop {
            match flight.poll() {
                DrainPoll::Done(output) => return *output,
                DrainPoll::Pending => std::thread::sleep(Duration::from_millis(1)),
            }
        }
    }

    /// Start one action batch without blocking (`try_send`s only):
    /// safe on the UI thread. Poll to `Done`; a newer batch drops the
    /// old flight (FIFO order keeps the newest view correct).
    pub fn debug_drain_begin(
        &self,
        actions: &[DebugAction],
        input: &DebugDrainInput,
    ) -> DrainFlight {
        self.debug_drain_begin_with_deadline(actions, input, DEBUG_DRAIN_TIMEOUT)
    }

    /// [`SessionHandle::debug_drain_begin`] with an explicit per-await
    /// deadline. Production uses the 5s UI budget; tests inject
    /// milliseconds to cover the timeout fallbacks without waiting.
    pub(crate) fn debug_drain_begin_with_deadline(
        &self,
        actions: &[DebugAction],
        input: &DebugDrainInput,
        deadline: Duration,
    ) -> DrainFlight {
        DrainFlight::begin(self.drain_ctx(), actions, input, deadline)
    }

    /// Start one images-only live fetch without blocking: only the
    /// cache lookup and a non-blocking send run here. Drive the
    /// returned flight with `poll` on the live tick until
    /// [`LiveImagesPoll::Done`]. Cache hits and coreless sessions
    /// resolve on the first poll.
    pub fn debug_live_images_begin(&self, need_images: bool) -> LiveImagesFlight {
        LiveImagesFlight::begin(self.drain_ctx().core, need_images, DEBUG_DRAIN_TIMEOUT)
    }
}

// ---------------------------------------------------------------------------
// Async drain flight
// ---------------------------------------------------------------------------

/// Per-await UI-thread budget. Same 5s as the blocking
/// `recv_debug_reply`: every await degrades to the identical fallback
/// instead of wedging input, so the worst case costs wall time but
/// never responsiveness.
pub(crate) const DEBUG_DRAIN_TIMEOUT: Duration = Duration::from_secs(5);

/// Poll result for [`DrainFlight`]: either the batch needs more ticks
/// or it finished with the render-ready display. The output rides
/// boxed: a finished drain crosses this boundary once, so there is
/// no reason to move ~240 bytes of display through every tick poll.
pub enum DrainPoll {
    Pending,
    Done(Box<DebugDrainOutput>),
}

/// Owned drain context: a cloned core (shared transport, metrics, and
/// image cache through `Arc`s) plus the shared write transaction.
/// No borrows, so a flight can live across UI ticks inside frontend
/// state; coherence with the session holds through the `Arc`s.
#[derive(Clone)]
pub struct DrainCtx {
    core: Option<EmuCore>,
    txn: Arc<Mutex<vm::WriteTransaction>>,
}

impl DrainCtx {
    pub(crate) fn new(core: Option<EmuCore>, txn: Arc<Mutex<vm::WriteTransaction>>) -> Self {
        Self { core, txn }
    }
}

/// Commit-phase remote ops, in batch order. Cancel is local (clear)
/// and runs inline while advancing; only these two cross the thread.
#[derive(Debug, Clone, Copy)]
enum CommitOp {
    Cancel,
    Confirm,
    RegWrite(u64),
}

/// Prepare-phase ops, in batch order. `Select` crosses the thread
/// (the old byte rides back); `Stage` is local.
#[derive(Debug, Clone, Copy)]
enum PrepOp {
    Select(u32),
    Stage(u64),
}

/// One outstanding thread reply with everything its resolution needs.
/// Exactly one exists at a time: phases run sequentially, mirroring
/// the blocking drain's round-trip order.
enum Outstanding {
    Step {
        rx: Receiver<Result<u64, DebuggerError>>,
        sent_at: Instant,
    },
    Commit {
        rx: Receiver<Result<(), DebuggerError>>,
        sent_at: Instant,
        confirm: bool,
    },
    Prepare {
        rx: Receiver<Result<InspectResult, InspectError>>,
        sent_at: Instant,
        space: SpaceId,
        addr: u32,
    },
    Freeze {
        rx: Receiver<Result<(), DebuggerError>>,
        sent_at: Instant,
    },
    Snapshot {
        rx: Receiver<Result<SnapshotResult, DebuggerError>>,
        sent_at: Instant,
    },
    Images {
        rx: Receiver<Result<Vec<DebugImage>, DebuggerError>>,
        sent_at: Instant,
    },
    Watch {
        rx: Receiver<Result<InspectResult, InspectError>>,
        sent_at: Instant,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FlightPhase {
    Exec,
    Commit,
    Prepare,
    Freeze,
    Snapshot,
    Images,
    Watch,
    Build,
}

/// Settled reply or still waiting. `Lost` covers both a dead thread
/// (disconnect, resolve now) and an over-deadline wait (same 5s
/// budget as the blocking drain, then the identical fallback).
enum Awaited<T> {
    Waiting,
    Ready(T),
    Lost,
}

/// One debugger batch as a tick-driven state machine. See the module
/// docs: same phase order and fallbacks as the blocking drain, but
/// `begin` only sends and `poll` only `try_recv`s.
pub struct DrainFlight {
    ctx: DrainCtx,
    deadline: Duration,
    phase: FlightPhase,
    outstanding: Option<Outstanding>,
    space: Option<SpaceId>,
    mem_addr: u32,
    dis_addr: Option<u32>,
    watch: Option<u32>,
    freeze: Option<(u32, u8)>,
    reg_target: Option<&'static str>,
    prev_dump_rows: Vec<HexRow>,
    need_images: bool,
    steps: VecDeque<StepUnit>,
    commits: VecDeque<CommitOp>,
    preps: VecDeque<PrepOp>,
    status: Option<vm::StatusMessage>,
    state_changing: bool,
    reselected: Option<(u32, Option<u8>)>,
    /// Confirmed write in flight (taken at send time for the status).
    confirm_sent: Option<(SpaceId, u32, u64)>,
    /// Register write in flight (target + value for the status).
    reg_sent: Option<(&'static str, u64)>,
    snap: Option<StoredSnapshot>,
    images_gen: u64,
    images: Vec<DebugImage>,
    watch_value: Option<u8>,
}

/// Snapshot sections stored at the snapshot phase for the build step.
/// `None` while unavailable (no core, refused inspect, lost reply):
/// the build then carries degenerates, exactly like the blocking
/// drain's early return.
#[derive(Default)]
struct StoredSnapshot {
    regs: Vec<(&'static str, u64)>,
    reg_names: Vec<&'static str>,
    dump_rows: Vec<HexRow>,
    panels: Vec<nerust_core_traits::debugger::DebugPanel>,
    disasm: Vec<nerust_core_traits::debugger::DisasmLine>,
}

enum Settle {
    Waiting(Outstanding),
    Settled,
}

impl DrainFlight {
    fn begin(
        ctx: DrainCtx,
        actions: &[DebugAction],
        input: &DebugDrainInput,
        deadline: Duration,
    ) -> Self {
        let mut flight = Self {
            ctx,
            deadline,
            phase: FlightPhase::Exec,
            outstanding: None,
            space: input.space,
            mem_addr: input.mem_addr,
            dis_addr: input.dis_addr,
            watch: input.watch,
            freeze: input.freeze,
            reg_target: input.reg_target,
            prev_dump_rows: input.prev_dump_rows.to_vec(),
            need_images: input.need_images,
            steps: VecDeque::new(),
            commits: VecDeque::new(),
            preps: VecDeque::new(),
            status: None,
            state_changing: false,
            reselected: None,
            confirm_sent: None,
            reg_sent: None,
            snap: None,
            images_gen: 0,
            images: Vec::new(),
            watch_value: None,
        };
        flight.classify(actions);
        flight
    }

    /// Single dispatch over the batch: fire-and-forget sends run now
    /// (they never wait), remote ops queue for their phases in batch
    /// order. One match, so a new action touches this pass only —
    /// not three parallel queues. Status wording mirrors
    /// `SessionHandle::debug_pause` and friends.
    fn classify(&mut self, actions: &[DebugAction]) {
        for action in actions {
            match action {
                DebugAction::Refresh | DebugAction::MemNav => {
                    self.state_changing = true;
                }
                DebugAction::SelectRow(addr) => {
                    self.state_changing = true;
                    self.preps.push_back(PrepOp::Select(*addr));
                }
                DebugAction::Pause => {
                    self.status = Some(self.ctx.pause_mirror(true));
                }
                DebugAction::Resume => {
                    self.status = Some(self.ctx.pause_mirror(false));
                }
                DebugAction::TogglePause => {
                    let paused = self.ctx.paused_mirror();
                    self.status = Some(self.ctx.pause_mirror(!paused));
                }
                DebugAction::StepFrame => {
                    self.state_changing = true;
                    self.steps.push_back(StepUnit::Frame);
                }
                DebugAction::StepInstr => {
                    self.state_changing = true;
                    self.steps.push_back(StepUnit::Instruction);
                }
                // Transaction traffic never kills: it builds the
                // transaction the commit phase consumes. Cancel clears
                // there; register writes carry no transaction state.
                DebugAction::EditStage(value) => {
                    self.preps.push_back(PrepOp::Stage(*value));
                }
                DebugAction::EditCancel => self.commits.push_back(CommitOp::Cancel),
                DebugAction::WriteConfirm => self.commits.push_back(CommitOp::Confirm),
                DebugAction::RegisterWrite(value) => {
                    self.commits.push_back(CommitOp::RegWrite(*value));
                }
            }
        }
    }

    /// Advance the flight without blocking: settle one reply or run
    /// one phase step. Local-only phases chain within the call, so a
    /// no-core batch completes on the first poll exactly like the
    /// old synchronous drain.
    pub fn poll(&mut self) -> DrainPoll {
        loop {
            if let Some(out) = self.outstanding.take() {
                match self.settle(out) {
                    Settle::Waiting(out) => {
                        self.outstanding = Some(out);
                        return DrainPoll::Pending;
                    }
                    Settle::Settled => {}
                }
                continue;
            }
            if self.phase == FlightPhase::Build {
                return DrainPoll::Done(Box::new(self.finish()));
            }
            self.advance_phase();
        }
    }

    fn settle(&mut self, out: Outstanding) -> Settle {
        // `try_recv` borrows, so a still-waiting reply rebuilds its
        // `Outstanding` untouched; resolutions consume it.
        match out {
            Outstanding::Step { rx, sent_at } => match await_reply(&rx, sent_at, self.deadline) {
                Awaited::Waiting => {
                    return Settle::Waiting(Outstanding::Step { rx, sent_at });
                }
                Awaited::Ready(reply) => self.resolve_step(reply),
                Awaited::Lost => self.resolve_step_lost(),
            },
            Outstanding::Commit {
                rx,
                sent_at,
                confirm,
            } => match await_reply(&rx, sent_at, self.deadline) {
                Awaited::Waiting => {
                    return Settle::Waiting(Outstanding::Commit {
                        rx,
                        sent_at,
                        confirm,
                    });
                }
                Awaited::Ready(reply) => self.resolve_commit(reply, confirm),
                Awaited::Lost => self.resolve_commit_lost(),
            },
            Outstanding::Prepare {
                rx,
                sent_at,
                space,
                addr,
            } => match await_reply(&rx, sent_at, self.deadline) {
                Awaited::Waiting => {
                    return Settle::Waiting(Outstanding::Prepare {
                        rx,
                        sent_at,
                        space,
                        addr,
                    });
                }
                Awaited::Ready(reply) => self.resolve_prepare(reply, space, addr),
                Awaited::Lost => self.resolve_prepare_lost(space, addr),
            },
            Outstanding::Freeze { rx, sent_at } => {
                match await_reply(&rx, sent_at, self.deadline) {
                    Awaited::Waiting => {
                        return Settle::Waiting(Outstanding::Freeze { rx, sent_at });
                    }
                    Awaited::Ready(_) | Awaited::Lost => {}
                }
                self.phase = FlightPhase::Snapshot;
            }
            Outstanding::Snapshot { rx, sent_at } => {
                match await_reply(&rx, sent_at, self.deadline) {
                    Awaited::Waiting => {
                        return Settle::Waiting(Outstanding::Snapshot { rx, sent_at });
                    }
                    Awaited::Ready(reply) => self.resolve_snapshot(reply),
                    Awaited::Lost => {}
                }
                self.phase = FlightPhase::Images;
            }
            Outstanding::Images { rx, sent_at } => {
                match await_reply(&rx, sent_at, self.deadline) {
                    Awaited::Waiting => {
                        return Settle::Waiting(Outstanding::Images { rx, sent_at });
                    }
                    Awaited::Ready(reply) => self.resolve_images(reply),
                    Awaited::Lost => {}
                }
                self.phase = FlightPhase::Watch;
            }
            Outstanding::Watch { rx, sent_at, .. } => {
                match await_reply(&rx, sent_at, self.deadline) {
                    Awaited::Waiting => {
                        return Settle::Waiting(Outstanding::Watch { rx, sent_at });
                    }
                    Awaited::Ready(reply) => self.watch_value = first_byte(reply),
                    Awaited::Lost => {}
                }
                self.phase = FlightPhase::Build;
            }
        }
        Settle::Settled
    }

    /// Run the current phase's local work and sends. Either parks on
    /// one reply (poll resumes there) or steps the phase forward, so
    /// the `poll` loop always terminates the call without blocking.
    fn advance_phase(&mut self) {
        match self.phase {
            FlightPhase::Exec => self.advance_exec(),
            FlightPhase::Commit => self.advance_commit(),
            FlightPhase::Prepare => self.advance_prepare(),
            FlightPhase::Freeze => self.advance_freeze(),
            FlightPhase::Snapshot => self.advance_snapshot(),
            FlightPhase::Images => self.advance_images(),
            FlightPhase::Watch => self.advance_watch(),
            FlightPhase::Build => {}
        }
    }

    fn advance_exec(&mut self) {
        let Some(unit) = self.steps.pop_front() else {
            self.phase = FlightPhase::Commit;
            return;
        };
        match self.ctx.step_send(unit) {
            Ok(rx) => {
                self.outstanding = Some(Outstanding::Step {
                    rx,
                    sent_at: Instant::now(),
                });
            }
            // No core / dead thread: today's `debug_step` reports
            // `NoCore` / `ThreadFailed`; keep draining the queue.
            Err(status) => {
                self.status = Some(status);
            }
        }
    }

    fn advance_commit(&mut self) {
        let Some(op) = self.commits.pop_front() else {
            // Stale-kill runs after commit, before prepare — same
            // seat as the blocking drain.
            if self.state_changing {
                self.ctx.txn.lock().unwrap().on_state_changing_traffic();
            }
            self.phase = FlightPhase::Prepare;
            return;
        };
        match op {
            CommitOp::Cancel => {
                self.ctx.txn.lock().unwrap().clear();
            }
            CommitOp::Confirm => self.send_confirm(),
            CommitOp::RegWrite(value) => self.send_reg_write(value),
        }
    }

    /// Commit entry: take the staged write (consuming, exactly once)
    /// and send it in the same non-blocking step, so superseding
    /// between ticks can never lose a confirmed write silently: the
    /// take only happens together with its send.
    fn send_confirm(&mut self) {
        use nerust_core_traits::debugger::MemoryWrite;
        let staged = self.ctx.txn.lock().unwrap().take_commit();
        let (space, addr, value) = match staged {
            Some(staged) => staged,
            None => {
                self.status = Some(vm::StatusMessage::NothingToWrite);
                return;
            }
        };
        self.confirm_sent = Some((space, addr, value));
        let sent = self.ctx.core.as_ref().map(|core| {
            core.write_memory_send(MemoryWrite {
                space,
                addr,
                width: 1,
                value,
            })
        });
        self.hold_commit(sent, true);
    }

    fn send_reg_write(&mut self, value: u64) {
        // No target: the blocking drain skips silently (no status).
        let Some(name) = self.reg_target else {
            return;
        };
        self.reg_sent = Some((name, value));
        let sent = self
            .ctx
            .core
            .as_ref()
            .map(|core| core.write_register_send(name, value));
        self.hold_commit(sent, false);
    }

    /// Park on a commit reply, or map a send failure to the same
    /// status the blocking drain reports (`NoCore` / `ThreadFailed`).
    fn hold_commit(
        &mut self,
        sent: Option<
            Result<Receiver<Result<(), DebuggerError>>, nerust_emu_thread::OperationError>,
        >,
        confirm: bool,
    ) {
        match sent {
            Some(Ok(rx)) => {
                self.outstanding = Some(Outstanding::Commit {
                    rx,
                    sent_at: Instant::now(),
                    confirm,
                });
            }
            Some(Err(error)) => {
                self.status = Some(vm::StatusMessage::ThreadFailed(format!("{error:?}")));
            }
            None => {
                self.status = Some(vm::StatusMessage::NoCore);
            }
        }
    }

    fn advance_prepare(&mut self) {
        let Some(op) = self.preps.pop_front() else {
            self.phase = FlightPhase::Freeze;
            return;
        };
        match op {
            PrepOp::Stage(value) => {
                self.ctx.txn.lock().unwrap().stage(value);
            }
            PrepOp::Select(addr) => self.send_prepare(addr),
        }
    }

    /// Selection prepare: the old byte rides back on the reply and
    /// feeds both the transaction and the selection display, exactly
    /// like `SessionHandle::debug_prepare_write`. Without a space the
    /// blocking drain skips the prepare but still reports the row.
    fn send_prepare(&mut self, addr: u32) {
        let Some(space) = self.space else {
            self.reselected = Some((addr, None));
            return;
        };
        let sent = self.ctx.core.as_ref().map(|core| {
            core.inspect_send(InspectRequest {
                space: Some(space),
                addr: Some(addr),
                rows: 1,
            })
        });
        match sent {
            Some(Ok(rx)) => {
                self.outstanding = Some(Outstanding::Prepare {
                    rx,
                    sent_at: Instant::now(),
                    space,
                    addr,
                });
            }
            // Send failure reads as empty, like the blocking path's
            // `None` old byte.
            Some(Err(_)) | None => {
                self.ctx.txn.lock().unwrap().prepare(space, addr, None);
                self.reselected = Some((addr, None));
            }
        }
    }

    /// Freeze re-apply: silent by design, but the blocking drain still
    /// waits for the reply, so the flight awaits it too (same
    /// deadline, result ignored).
    fn advance_freeze(&mut self) {
        use nerust_core_traits::debugger::MemoryWrite;
        let (Some((addr, value)), Some(space)) = (self.freeze, self.space) else {
            self.phase = FlightPhase::Snapshot;
            return;
        };
        let sent = self.ctx.core.as_ref().map(|core| {
            core.write_memory_send(MemoryWrite {
                space,
                addr,
                width: 1,
                value: u64::from(value),
            })
        });
        match sent {
            Some(Ok(rx)) => {
                self.outstanding = Some(Outstanding::Freeze {
                    rx,
                    sent_at: Instant::now(),
                });
            }
            // Send failure is silent here, like the blocking drain's
            // ignored `bool`.
            Some(Err(_)) | None => {
                self.phase = FlightPhase::Snapshot;
            }
        }
    }

    fn advance_snapshot(&mut self) {
        let sent = self.ctx.core.as_ref().map(|core| {
            core.snapshot_batch_send(nerust_core_traits::debugger::SnapshotRequest {
                inspect: InspectRequest {
                    space: self.space,
                    addr: self.space.map(|_| self.mem_addr),
                    rows: DEBUG_DUMP_PAGE_ROWS,
                },
                anchor: self.dis_addr,
                disasm_rows: DEBUG_DISASM_ROWS,
            })
        });
        match sent {
            Some(Ok(rx)) => {
                self.outstanding = Some(Outstanding::Snapshot {
                    rx,
                    sent_at: Instant::now(),
                });
            }
            // Unavailable core / dead thread: degenerate sections,
            // like the blocking drain's early return.
            Some(Err(_)) | None => {
                self.phase = FlightPhase::Images;
            }
        }
    }

    fn advance_images(&mut self) {
        if !self.need_images {
            self.phase = FlightPhase::Watch;
            return;
        }
        let Some(core) = self.ctx.core.as_ref() else {
            self.phase = FlightPhase::Watch;
            return;
        };
        // A running core advances memory at any time: dirty first so
        // the lookup below always refetches (mirrors `debug_images`).
        if !core.metrics().paused {
            core.mark_images_dirty();
        }
        if let Some((generation, images)) = core.images_cache_lookup() {
            self.images_gen = generation;
            self.images = images;
            self.phase = FlightPhase::Watch;
            return;
        }
        match core.images_send() {
            Ok(rx) => {
                self.outstanding = Some(Outstanding::Images {
                    rx,
                    sent_at: Instant::now(),
                });
            }
            Err(_) => {
                self.phase = FlightPhase::Watch;
            }
        }
    }

    /// Watch prefers the folded page read; off-page watches fall back
    /// to one single-byte inspect — same order as the blocking drain.
    fn advance_watch(&mut self) {
        let watch = self.watch;
        let folded = watch.and_then(|addr| self.fold_watch(addr));
        if let Some(value) = folded {
            self.watch_value = Some(value);
            self.phase = FlightPhase::Build;
            return;
        }
        let (Some(addr), Some(space)) = (watch, self.space) else {
            self.phase = FlightPhase::Build;
            return;
        };
        let sent = self.ctx.core.as_ref().map(|core| {
            core.inspect_send(InspectRequest {
                space: Some(space),
                addr: Some(addr),
                rows: 1,
            })
        });
        match sent {
            Some(Ok(rx)) => {
                self.outstanding = Some(Outstanding::Watch {
                    rx,
                    sent_at: Instant::now(),
                });
            }
            Some(Err(_)) | None => {
                self.phase = FlightPhase::Build;
            }
        }
    }

    fn fold_watch(&self, addr: u32) -> Option<u8> {
        let snap = self.snap.as_ref()?;
        snap.dump_rows
            .iter()
            .find(|row| addr >= row.addr && addr < row.addr + u32::from(row.valid))
            .map(|row| row.bytes[(addr - row.addr) as usize])
    }

    fn resolve_step(&mut self, reply: Result<u64, DebuggerError>) {
        match reply {
            Ok(count) => {
                if let Some(core) = self.ctx.core.as_ref() {
                    core.mark_images_dirty();
                }
                self.status = Some(vm::StatusMessage::Stepped(count));
            }
            Err(error) => {
                self.status = Some(vm::StatusMessage::StepFailed(format!("{error:?}")));
            }
        }
    }

    /// Lost step reply (dead thread / over deadline): the blocking
    /// drain's `recv_timeout` maps this to `ThreadFailed(NoReply)`.
    fn resolve_step_lost(&mut self) {
        self.status = Some(vm::StatusMessage::ThreadFailed("NoReply".to_string()));
    }

    fn resolve_commit(&mut self, reply: Result<(), DebuggerError>, confirm: bool) {
        if matches!(reply, Ok(()))
            && let Some(core) = self.ctx.core.as_ref()
        {
            core.mark_images_dirty();
        }
        self.status = Some(match (confirm, reply) {
            (true, Ok(())) => {
                let (_, addr, value) = self.confirm_sent.unwrap_or((SpaceId(0), 0, 0));
                vm::StatusMessage::WroteMemory { addr, value }
            }
            (true, Err(error)) => vm::StatusMessage::WriteRefused(format!("{error:?}")),
            (false, Ok(())) => {
                let (name, value) = self.reg_sent.unwrap_or(("?", 0));
                vm::StatusMessage::RegisterWrote { name, value }
            }
            (false, Err(error)) => vm::StatusMessage::RegisterRefused(format!("{error:?}")),
        });
    }

    /// Lost commit reply: same `ThreadFailed(NoReply)` fallback as
    /// the blocking drain's timeout path (both ops share it).
    fn resolve_commit_lost(&mut self) {
        self.status = Some(vm::StatusMessage::ThreadFailed("NoReply".to_string()));
    }

    fn resolve_prepare(
        &mut self,
        reply: Result<InspectResult, InspectError>,
        space: SpaceId,
        addr: u32,
    ) {
        let old = first_byte(reply);
        self.ctx.txn.lock().unwrap().prepare(space, addr, old);
        self.reselected = Some((addr, old));
    }

    fn resolve_prepare_lost(&mut self, space: SpaceId, addr: u32) {
        self.ctx.txn.lock().unwrap().prepare(space, addr, None);
        self.reselected = Some((addr, None));
    }

    fn resolve_snapshot(&mut self, reply: Result<SnapshotResult, DebuggerError>) {
        // Failure semantics mirror the blocking drain: an inspect
        // failure skips the rest (degenerate sections).
        let batch = match reply {
            Ok(batch) => batch,
            Err(_) => return,
        };
        let inspected = match batch.inspect {
            Ok(inspected) => inspected,
            Err(_) => return,
        };
        self.snap = Some(StoredSnapshot {
            regs: inspected
                .registers
                .iter()
                .map(|&(name, value)| (name, value))
                .collect(),
            reg_names: inspected.registers.iter().map(|(name, _)| *name).collect(),
            dump_rows: inspected.dump.rows.iter().copied().collect(),
            panels: inspected.panels.iter().cloned().collect(),
            disasm: batch.disasm,
        });
    }

    fn resolve_images(&mut self, reply: Result<Vec<DebugImage>, DebuggerError>) {
        let images = match reply {
            Ok(images) => images,
            Err(_) => return,
        };
        if let Some(core) = self.ctx.core.as_ref() {
            let stored = core.images_store(images);
            // Unchanged content reports the served generation: skip
            // the RGBA conversion too, not just the viewer re-derive
            // — the frontend generation guard drops the empty pixels.
            self.images_gen = stored.generation;
            self.images = if stored.fresh {
                stored.images
            } else {
                Vec::new()
            };
        }
    }

    fn finish(&mut self) -> DebugDrainOutput {
        // Move out of the stored snapshot: the flight is done, so the
        // vecs travel instead of cloning (same output, no second copy).
        let snap = self.snap.take().unwrap_or_default();
        let paused = self.ctx.paused_mirror();
        let status = self.status.take().unwrap_or(if paused {
            vm::StatusMessage::Paused
        } else {
            vm::StatusMessage::Running
        });
        let display = DisplayCache {
            regs: snap.regs,
            diff: vm::diff_rows(&self.prev_dump_rows, &snap.dump_rows),
            dump_rows: snap.dump_rows,
            disasm_lines: snap.disasm,
            panels: snap.panels,
            images: std::mem::take(&mut self.images)
                .iter()
                .map(vm::rgba_image)
                .collect(),
            images_gen: self.images_gen,
            watch_value: self.watch_value,
            paused,
            reg_names: snap.reg_names,
            status,
            pending: self.ctx.txn.lock().unwrap().pending(),
        };
        DebugDrainOutput {
            display,
            reselected: self.reselected,
        }
    }
}

impl DrainCtx {
    /// Pause truth without blocking: no core reads as paused, like
    /// `SessionHandle::debug_paused`.
    fn paused_mirror(&self) -> bool {
        self.core.as_ref().is_none_or(|core| core.metrics().paused)
    }

    /// Fire-and-forget pause/resume with the same status wording as
    /// `SessionHandle::debug_pause` / `debug_resume`. Sends never
    /// block (`try_send`); only the metrics lock is touched.
    fn pause_mirror(&self, pause: bool) -> vm::StatusMessage {
        let Some(core) = self.core.as_ref() else {
            return vm::StatusMessage::NoCore;
        };
        let result = if pause { core.pause() } else { core.resume() };
        match result {
            Ok(()) if pause => vm::StatusMessage::Paused,
            Ok(()) => vm::StatusMessage::Resume,
            Err(error) if pause => vm::StatusMessage::PauseFailed(format!("{error:?}")),
            Err(error) => vm::StatusMessage::ResumeFailed(format!("{error:?}")),
        }
    }

    /// Queue one step without blocking. The `Err` status mirrors
    /// `SessionHandle::debug_step`'s no-core path; thread failures
    /// surface at reply time instead.
    fn step_send(
        &self,
        unit: StepUnit,
    ) -> Result<Receiver<Result<u64, DebuggerError>>, vm::StatusMessage> {
        match self.core.as_ref() {
            Some(core) => core
                .step_send(unit)
                .map_err(|error| vm::StatusMessage::ThreadFailed(format!("{error:?}"))),
            None => Err(vm::StatusMessage::NoCore),
        }
    }
}

/// Non-blocking reply poll with the drain deadline. `Lost` covers a
/// dead thread (resolve now, silently) and an over-deadline wait
/// (same warn + fallback as the blocking `recv_debug_reply`).
fn await_reply<T>(rx: &Receiver<T>, sent_at: Instant, deadline: Duration) -> Awaited<T> {
    match rx.try_recv() {
        Ok(reply) => Awaited::Ready(reply),
        Err(std::sync::mpsc::TryRecvError::Disconnected) => Awaited::Lost,
        Err(std::sync::mpsc::TryRecvError::Empty) => {
            if sent_at.elapsed() > deadline {
                log::warn!("debugger round-trip timed out; emu thread may be stalled");
                Awaited::Lost
            } else {
                Awaited::Waiting
            }
        }
    }
}

/// Old-byte / watch-value extraction, mirroring
/// `SessionHandle::debug_read_byte`: the first row's first byte when
/// valid, `None` on any refusal.
fn first_byte(reply: Result<InspectResult, InspectError>) -> Option<u8> {
    let inspected = reply.ok()?;
    let row = inspected.dump.rows.first()?;
    (row.valid > 0).then_some(row.bytes[0])
}

// ---------------------------------------------------------------------------
// Async live-images flight
// ---------------------------------------------------------------------------

/// Poll result for [`LiveImagesFlight`]: either the fetch needs more
/// ticks or it finished (`None` when imageless or failed, like the
/// blocking refresh).
pub enum LiveImagesPoll {
    Pending,
    Done(crate::session::LiveImages),
}

/// One images-only fetch as a tick-driven state machine. `begin`
/// sends without blocking; `poll` receives without blocking. Cache
/// hits and coreless sessions resolve on the first poll.
pub struct LiveImagesFlight {
    core: Option<EmuCore>,
    deadline: Duration,
    state: Option<LiveImagesState>,
}

enum LiveImagesState {
    Fetching {
        rx: Receiver<Result<Vec<DebugImage>, DebuggerError>>,
        sent_at: Instant,
    },
    Ready(crate::session::LiveImages),
}

impl LiveImagesFlight {
    fn begin(core: Option<EmuCore>, need_images: bool, deadline: Duration) -> Self {
        if !need_images {
            return Self::ready(None, deadline);
        }
        let Some(core) = core else {
            return Self::ready(None, deadline);
        };
        // A running core advances memory at any time: dirty first so
        // the lookup below refetches (mirrors DrainFlight and
        // EmuCore::debug_images). Without this the first stored decode
        // serves forever and live follow freezes on one frame.
        if !core.metrics().paused {
            core.mark_images_dirty();
        }
        if let Some((generation, images)) = core.images_cache_lookup() {
            return Self {
                core: Some(core),
                deadline,
                state: Some(LiveImagesState::Ready(Some((
                    generation,
                    images.iter().map(vm::rgba_image).collect(),
                )))),
            };
        }
        match core.images_send() {
            Ok(rx) => Self {
                core: Some(core),
                deadline,
                state: Some(LiveImagesState::Fetching {
                    rx,
                    sent_at: Instant::now(),
                }),
            },
            Err(_) => Self::ready(None, deadline),
        }
    }

    fn ready(live: crate::session::LiveImages, deadline: Duration) -> Self {
        Self {
            core: None,
            deadline,
            state: Some(LiveImagesState::Ready(live)),
        }
    }

    /// Advance the fetch without blocking: at most one `try_recv`.
    /// A still-waiting fetch stays stored; anything else resolves to
    /// `Done`, so orphaned flights from a closed viewer settle
    /// harmlessly on the next tick.
    pub fn poll(&mut self) -> LiveImagesPoll {
        let state = self.state.take();
        match state {
            None | Some(LiveImagesState::Ready(None)) => LiveImagesPoll::Done(None),
            Some(LiveImagesState::Ready(live)) => LiveImagesPoll::Done(live),
            Some(LiveImagesState::Fetching { rx, sent_at }) => {
                match await_reply(&rx, sent_at, self.deadline) {
                    Awaited::Waiting => {
                        self.state = Some(LiveImagesState::Fetching { rx, sent_at });
                        LiveImagesPoll::Pending
                    }
                    Awaited::Ready(reply) => LiveImagesPoll::Done(self.store(reply)),
                    Awaited::Lost => LiveImagesPoll::Done(None),
                }
            }
        }
    }

    fn store(&self, reply: Result<Vec<DebugImage>, DebuggerError>) -> crate::session::LiveImages {
        let images = reply.ok()?;
        let core = self.core.as_ref()?;
        let stored = core.images_store(images);
        // Unchanged content skips the RGBA conversion like the drain
        // flight: the served generation makes viewers drop it.
        let items = if stored.fresh {
            stored.images.iter().map(vm::rgba_image).collect()
        } else {
            Vec::new()
        };
        Some((stored.generation, items))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn await_reply_waits_readies_and_loses() {
        use std::sync::mpsc;
        use std::time::Duration;
        // Empty + fresh: still waiting, never blocking.
        let (_tx, rx) = mpsc::channel::<u8>();
        assert!(matches!(
            await_reply(&rx, Instant::now(), Duration::from_secs(5)),
            Awaited::Waiting
        ));
        // A queued reply resolves immediately.
        let (tx, rx) = mpsc::channel::<u8>();
        tx.send(7).unwrap();
        assert!(matches!(
            await_reply(&rx, Instant::now(), Duration::ZERO),
            Awaited::Ready(7)
        ));
        // Dropped sender: lost now, not at the deadline.
        let (tx, rx) = mpsc::channel::<u8>();
        drop(tx);
        assert!(matches!(
            await_reply(&rx, Instant::now(), Duration::from_secs(5)),
            Awaited::Lost
        ));
        // Stale wait past the deadline: lost with the same fallback
        // as the blocking drain's timeout (warns, then degrades).
        let (_tx, rx) = mpsc::channel::<u8>();
        let past = Instant::now() - Duration::from_secs(6);
        assert!(matches!(
            await_reply(&rx, past, Duration::from_secs(5)),
            Awaited::Lost
        ));
    }
}
