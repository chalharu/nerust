//! Shared debugger action drain: one phase order for every frontend.
//! Blocking wrapper spins flights; frontends use begin-plus-poll.

use std::{
    collections::VecDeque,
    sync::{Arc, Mutex, MutexGuard, mpsc::Receiver},
    time::{Duration, Instant},
};

use nerust_core_traits::debugger::{
    DEBUG_DISASM_ROWS, DEBUG_DUMP_PAGE_ROWS, DebugImage, DebuggerError, HexRow, InspectError,
    InspectRequest, InspectResult, SnapshotResult, SpaceId, StepUnit,
};
use nerust_gui_viewmodel::debugger as debug_vm;
use nerust_gui_viewmodel::debugger::{self as vm, DisplayCache};

use crate::{emu_core::EmuCore, emu_core::LiveDebugPort, session::SessionHandle};

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

/// Typed debugger failure for one thread round-trip: transport vs refusal.
/// Exactly one place formats each kind; no `format!("{error:?}")` at call sites.
#[derive(Debug, Clone)]
pub(crate) enum CoreFailure {
    /// No live core behind the session.
    NoCore,
    /// The emu thread is gone or never answered.
    TransportGone,
    /// The core refused the request; the detail rides verbatim.
    Refused(String),
}

impl From<nerust_emu_thread::OperationError> for CoreFailure {
    fn from(error: nerust_emu_thread::OperationError) -> Self {
        use nerust_emu_thread::OperationError as Op;
        match error {
            Op::WorkerUnavailable | Op::NoReply => Self::TransportGone,
            Op::Reply(detail) => Self::Refused(detail),
        }
    }
}

impl From<nerust_core_traits::debugger::DebuggerError> for CoreFailure {
    fn from(error: nerust_core_traits::debugger::DebuggerError) -> Self {
        // `DebuggerError` is data-only; the pinned strings render here alone, never at call sites.
        Self::Refused(format!("{error:?}"))
    }
}

impl From<nerust_core_traits::debugger::InspectError> for CoreFailure {
    fn from(error: nerust_core_traits::debugger::InspectError) -> Self {
        // Same data-only constraint as `DebuggerError` above.
        Self::Refused(format!("{error:?}"))
    }
}

impl CoreFailure {
    pub(crate) fn no_core() -> Self {
        Self::NoCore
    }

    pub(crate) fn thread_status(self) -> debug_vm::StatusMessage {
        match self {
            Self::NoCore => debug_vm::StatusMessage::NoCore,
            Self::TransportGone => {
                debug_vm::StatusMessage::ThreadFailed("transport unavailable".to_string())
            }
            Self::Refused(detail) => debug_vm::StatusMessage::ThreadFailed(detail),
        }
    }

    pub(crate) fn register_status(self) -> debug_vm::StatusMessage {
        match self {
            Self::NoCore => debug_vm::StatusMessage::NoCore,
            Self::TransportGone => {
                debug_vm::StatusMessage::ThreadFailed("transport unavailable".to_string())
            }
            Self::Refused(detail) => {
                if detail.is_empty() {
                    debug_vm::StatusMessage::RegisterRefused("refused".to_string())
                } else {
                    debug_vm::StatusMessage::RegisterRefused(detail)
                }
            }
        }
    }

    pub(crate) fn write_status(self) -> debug_vm::StatusMessage {
        match self {
            Self::NoCore => debug_vm::StatusMessage::NoCore,
            Self::TransportGone => {
                debug_vm::StatusMessage::ThreadFailed("transport unavailable".to_string())
            }
            Self::Refused(detail) => debug_vm::StatusMessage::WriteRefused(detail),
        }
    }

    pub(crate) fn pause_status(self, pausing: bool) -> debug_vm::StatusMessage {
        match (self, pausing) {
            (Self::NoCore, _) => debug_vm::StatusMessage::NoCore,
            (Self::TransportGone, _) => {
                debug_vm::StatusMessage::ThreadFailed("transport unavailable".to_string())
            }
            (Self::Refused(detail), true) => debug_vm::StatusMessage::PauseFailed(detail),
            (Self::Refused(detail), false) => debug_vm::StatusMessage::ResumeFailed(detail),
        }
    }

    pub(crate) fn step_status(self) -> debug_vm::StatusMessage {
        match self {
            Self::NoCore => debug_vm::StatusMessage::NoCore,
            Self::TransportGone => {
                debug_vm::StatusMessage::ThreadFailed("transport unavailable".to_string())
            }
            Self::Refused(detail) => debug_vm::StatusMessage::StepFailed(detail),
        }
    }
}

/// Whether a debugger pass fetches system images (pattern tables).
/// Frontend structs keep `bool` fields and convert at the drain boundary via `From<bool>`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ImageNeed {
    /// Skip the fetch (no PPU viewer open): CHR decode is skipped,
    /// not just unused.
    #[default]
    Skip,
    /// Fetch system images through the emu thread.
    Fetch,
}

impl From<bool> for ImageNeed {
    fn from(need_images: bool) -> Self {
        if need_images { Self::Fetch } else { Self::Skip }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrafficKind {
    /// Step/refresh/pause traffic: kills staged-but-uncommitted values.
    StateChanging,
    /// Select/stage/commit traffic: builds the transaction in place.
    Transactional,
}

impl From<bool> for TrafficKind {
    fn from(state_changing: bool) -> Self {
        if state_changing {
            Self::StateChanging
        } else {
            Self::Transactional
        }
    }
}

impl SessionHandle {
    /// Run one action batch and return the render-ready display.
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

    /// Start one action batch without blocking; safe on the UI thread.
    /// Poll to `Done`; a newer batch drops the old flight, newest view wins.
    pub fn debug_drain_begin(
        &self,
        actions: &[DebugAction],
        input: &DebugDrainInput,
    ) -> DrainFlight {
        self.debug_drain_begin_with_deadline(actions, input, DEBUG_DRAIN_TIMEOUT)
    }

    /// [`SessionHandle::debug_drain_begin`] with an explicit per-await deadline.
    /// Tests inject milliseconds to cover the timeout fallbacks without waiting.
    pub(crate) fn debug_drain_begin_with_deadline(
        &self,
        actions: &[DebugAction],
        input: &DebugDrainInput,
        deadline: Duration,
    ) -> DrainFlight {
        DrainFlight::begin(self.drain_ctx(), actions, input, deadline)
    }

    /// Start one images-only live fetch without blocking; drive the flight with `poll` per tick.
    /// Cache hits and coreless sessions resolve on the first poll.
    pub fn debug_live_images_begin(&self, need_images: bool) -> LiveImagesFlight {
        LiveImagesFlight::begin(
            self.drain_ctx().core,
            ImageNeed::from(need_images),
            DEBUG_DRAIN_TIMEOUT,
        )
    }
}

/// Per-await UI-thread budget; every await degrades to the identical fallback.
/// The worst case costs wall time but never responsiveness.
pub(crate) const DEBUG_DRAIN_TIMEOUT: Duration = Duration::from_secs(5);

/// Poll result for [`DrainFlight`]: either the batch needs more ticks
/// or it finished with the render-ready display (boxed).
pub enum DrainPoll {
    Pending,
    Done(Box<DebugDrainOutput>),
}

/// Owned drain context: cloned core plus the shared write transaction, no borrows.
/// A flight can live across UI ticks inside frontend state.
#[derive(Clone)]
pub struct DrainCtx {
    core: Option<EmuCore>,
    txn: Arc<Mutex<vm::WriteTransaction>>,
}

impl DrainCtx {
    pub(crate) fn new(core: Option<EmuCore>, txn: Arc<Mutex<vm::WriteTransaction>>) -> Self {
        Self { core, txn }
    }

    /// Narrow debug port behind the owned core; flights program against this.
    pub fn core_port(&self) -> Option<&dyn LiveDebugPort> {
        self.core.as_ref().map(|core| core as &dyn LiveDebugPort)
    }

    /// Poison-recovering transaction lock: plain data survives a panicked holder.
    /// Exactly one logging point for the recovery.
    fn lock_txn(&self) -> MutexGuard<'_, vm::WriteTransaction> {
        self.txn.lock().unwrap_or_else(|error| {
            log::error!("debug transaction mutex poisoned, recovering");
            error.into_inner()
        })
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
/// Exactly one exists at a time: phases run sequentially.
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

/// Settled reply or still waiting; `Lost` resolves now on disconnect or over-deadline wait.
enum Awaited<T> {
    Waiting,
    Ready(T),
    Lost,
}

/// One debugger batch as a tick-driven state machine: `begin` sends, `poll` receives.
/// Same phase order and fallbacks as the blocking drain.
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
    need_images: ImageNeed,
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
/// `None` while unavailable; the build then carries degenerates.
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
            need_images: ImageNeed::from(input.need_images),
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

    /// Single dispatch over the batch: fire-and-forget sends run now, remote ops queue in order.
    /// A new action touches this pass only, not three parallel queues.
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
                // Transaction traffic builds what commit consumes; register writes carry none.
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

    /// Advance the flight without blocking: settle one reply or run one phase step.
    /// A no-core batch completes on the first poll, like the old synchronous drain.
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

    /// Run the current phase local work and sends, parking on one reply or stepping forward.
    /// The `poll` loop always terminates the call without blocking.
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
                self.ctx.lock_txn().on_state_changing_traffic();
            }
            self.phase = FlightPhase::Prepare;
            return;
        };
        match op {
            CommitOp::Cancel => {
                self.ctx.lock_txn().clear();
            }
            CommitOp::Confirm => self.send_confirm(),
            CommitOp::RegWrite(value) => self.send_reg_write(value),
        }
    }

    /// Commit entry: take the staged write and send it in the same non-blocking step.
    /// The take only happens together with its send, so a confirmed write is never lost silently.
    fn send_confirm(&mut self) {
        use nerust_core_traits::debugger::MemoryWrite;
        let staged = self.ctx.lock_txn().take_commit();
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
                self.status = Some(CoreFailure::from(error).thread_status());
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
                self.ctx.lock_txn().stage(value);
            }
            PrepOp::Select(addr) => self.send_prepare(addr),
        }
    }

    /// Selection prepare: the old byte rides back on the reply for the transaction and display.
    /// Without a space the blocking drain skips the prepare but still reports the row.
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
                self.ctx.lock_txn().prepare(space, addr, None);
                self.reselected = Some((addr, None));
            }
        }
    }

    /// Freeze re-apply: the flight awaits the reply too, with the result ignored.
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
        if self.need_images == ImageNeed::Skip {
            self.phase = FlightPhase::Watch;
            return;
        }
        let Some(core) = self.ctx.core.as_ref() else {
            self.phase = FlightPhase::Watch;
            return;
        };
        // A running core advances memory at any time: dirty first so the
        // lookup below always refetches (mirrors `debug_images`).
        if !core.metrics().paused {
            core.mark_images_dirty();
        }
        if let Some((generation, shared)) = core.images_cache_lookup() {
            self.images_gen = generation;
            self.images = Vec::clone(&shared);
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
        crate::session::debug_snapshot::fold_watch_byte(&snap.dump_rows, addr)
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
                self.status = Some(CoreFailure::from(error).step_status());
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
            (true, Err(error)) => CoreFailure::from(error).write_status(),
            (false, Ok(())) => {
                let (name, value) = self.reg_sent.unwrap_or(("?", 0));
                vm::StatusMessage::RegisterWrote { name, value }
            }
            (false, Err(error)) => CoreFailure::from(error).register_status(),
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
        self.ctx.lock_txn().prepare(space, addr, old);
        self.reselected = Some((addr, old));
    }

    fn resolve_prepare_lost(&mut self, space: SpaceId, addr: u32) {
        self.ctx.lock_txn().prepare(space, addr, None);
        self.reselected = Some((addr, None));
    }

    fn resolve_snapshot(&mut self, reply: Result<SnapshotResult, DebuggerError>) {
        // Failure semantics mirror the blocking drain: an inspect
        // failure skips the rest (degenerate sections).
        let batch = match reply {
            Ok(batch) => batch,
            Err(_) => return,
        };
        let Some(parts) = crate::session::debug_snapshot::assemble_snapshot_parts(batch) else {
            return;
        };
        self.snap = Some(StoredSnapshot {
            regs: parts.regs,
            reg_names: parts.reg_names,
            dump_rows: parts.dump_rows,
            panels: parts.panels,
            disasm: parts.disasm,
        });
    }

    fn resolve_images(&mut self, reply: Result<Vec<DebugImage>, DebuggerError>) {
        let images = match reply {
            Ok(images) => images,
            Err(_) => return,
        };
        if let Some(core) = self.ctx.core.as_ref() {
            let stored = core.images_store(images);
            // Unchanged content reports the served generation: skip the RGBA conversion too.
            self.images_gen = stored.generation;
            self.images = if stored.fresh {
                stored.images
            } else {
                Vec::new()
            };
        }
    }

    fn finish(&mut self) -> DebugDrainOutput {
        // Domain snapshot first; the display adapter renders it against the previous rows.
        let snapshot = self.finish_snapshot();
        let reselected = snapshot.reselected;
        let display = snapshot.into_display(&self.prev_dump_rows);
        DebugDrainOutput {
            display,
            reselected,
        }
    }

    /// Domain snapshot for one finished batch: moves the stored sections out, no cloning.
    fn finish_snapshot(&mut self) -> DrainSnapshot {
        // Move out of the stored snapshot: the flight is done, so the
        // vecs travel instead of cloning (same output, no second copy).
        let snap = self.snap.take().unwrap_or_default();
        let paused = self.ctx.paused_mirror();
        let status = self.status.take().unwrap_or(if paused {
            vm::StatusMessage::Paused
        } else {
            vm::StatusMessage::Running
        });
        DrainSnapshot {
            regs: snap.regs,
            reg_names: snap.reg_names,
            dump_rows: snap.dump_rows,
            panels: snap.panels,
            disasm: snap.disasm,
            images: std::mem::take(&mut self.images),
            images_gen: self.images_gen,
            watch_value: self.watch_value,
            paused,
            status,
            pending: self.ctx.lock_txn().pending(),
            reselected: self.reselected,
        }
    }
}

/// Domain snapshot for one drain batch: structured sections, no rendering decisions.
/// The display adapter consumes this; the flight never formats.
pub struct DrainSnapshot {
    /// Register `(name, value)` pairs in thread order.
    pub regs: Vec<(&'static str, u64)>,
    /// Register names in thread order.
    pub reg_names: Vec<&'static str>,
    /// Memory dump rows.
    pub dump_rows: Vec<HexRow>,
    /// Debugger panels.
    pub panels: Vec<nerust_core_traits::debugger::DebugPanel>,
    /// Disassembly lines at the anchor.
    pub disasm: Vec<nerust_core_traits::debugger::DisasmLine>,
    /// Fetched system images (empty when unchanged or skipped).
    pub images: Vec<DebugImage>,
    /// Source generation of the served images.
    pub images_gen: u64,
    /// Folded watch value, if the watch landed on the page or the fallback read.
    pub watch_value: Option<u8>,
    /// Pause truth behind this batch.
    pub paused: bool,
    /// Batch status for the status line.
    pub status: vm::StatusMessage,
    /// Staged but uncommitted write, if any.
    pub pending: Option<vm::PendingWrite>,
    /// Fresh `(address, old byte)` when the batch selected a row.
    pub reselected: Option<(u32, Option<u8>)>,
}

impl DrainSnapshot {
    /// Render-ready display for one domain snapshot, diffed against the previous dump rows.
    pub fn into_display(self, prev_dump_rows: &[HexRow]) -> DisplayCache {
        DisplayCache {
            regs: self.regs,
            diff: vm::diff_rows(prev_dump_rows, &self.dump_rows),
            dump_rows: self.dump_rows,
            disasm_lines: self.disasm,
            panels: self.panels,
            images: self.images.iter().map(vm::rgba_image).collect(),
            images_gen: self.images_gen,
            watch_value: self.watch_value,
            paused: self.paused,
            reg_names: self.reg_names,
            status: self.status,
            pending: self.pending,
        }
    }
}

impl DrainCtx {
    /// Pause truth without blocking: no core reads as paused, like
    /// `SessionHandle::debug_paused`.
    fn paused_mirror(&self) -> bool {
        self.core.as_ref().is_none_or(|core| core.metrics().paused)
    }

    /// Fire-and-forget pause/resume with the same status wording as the session pause/resume.
    /// Sends never block; only the metrics lock is touched.
    fn pause_mirror(&self, pause: bool) -> vm::StatusMessage {
        let Some(core) = self.core.as_ref() else {
            return vm::StatusMessage::NoCore;
        };
        let result = if pause { core.pause() } else { core.resume() };
        match result {
            Ok(()) if pause => vm::StatusMessage::Paused,
            Ok(()) => vm::StatusMessage::Resume,
            Err(error) => CoreFailure::from(error).pause_status(pause),
        }
    }

    /// Queue one step without blocking through the narrow port.
    fn step_send(
        &self,
        unit: StepUnit,
    ) -> Result<Receiver<Result<u64, DebuggerError>>, vm::StatusMessage> {
        match self.core_port() {
            Some(port) => port
                .step_send(unit)
                .map_err(|error| CoreFailure::from(error).thread_status()),
            None => Err(vm::StatusMessage::NoCore),
        }
    }
}

/// Non-blocking reply poll with the drain deadline; `Lost` resolves now or falls back on timeout.
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

/// Old-byte / watch-value extraction: the first row's first byte when valid, `None` on refusal.
fn first_byte(reply: Result<InspectResult, InspectError>) -> Option<u8> {
    let inspected = reply.ok()?;
    let row = inspected.dump.rows.first()?;
    (row.valid > 0).then_some(row.bytes[0])
}

/// Poll result for [`LiveImagesFlight`]: needs more ticks, or finished (`None` when imageless).
pub enum LiveImagesPoll {
    Pending,
    Done(crate::session::LiveImages),
}

/// One images-only fetch as a tick-driven state machine: non-blocking send, non-blocking poll.
/// Cache hits and coreless sessions resolve on the first poll.
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
    fn begin(core: Option<EmuCore>, need_images: ImageNeed, deadline: Duration) -> Self {
        if need_images == ImageNeed::Skip {
            return Self::ready(None, deadline);
        }
        let Some(core) = core else {
            return Self::ready(None, deadline);
        };
        // Dirty first on a running core so the lookup below refetches; memory may advance anytime.
        // Without this the first stored decode serves forever and live follow freezes.
        if !core.metrics().paused {
            core.mark_images_dirty();
        }
        if let Some((generation, shared)) = core.images_cache_lookup() {
            return Self {
                core: Some(core),
                deadline,
                state: Some(LiveImagesState::Ready(Some((
                    generation,
                    shared.iter().map(vm::rgba_image).collect(),
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
    /// Anything but still-waiting resolves to `Done`, so orphaned flights settle harmlessly.
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

#[cfg(test)]
mod port_tests {
    use crate::debug::*;

    #[test]
    fn drain_ctx_exposes_narrow_debug_port() {
        use crate::session::test_util::test_session;
        let session = test_session();
        let ctx = session.drain_ctx();
        assert!(ctx.core_port().is_some());
        let empty = DrainCtx::new(None, Arc::new(Mutex::new(vm::WriteTransaction::Empty)));
        assert!(empty.core_port().is_none());
    }
}

#[cfg(test)]
mod debug_cov_tests {
    use crate::debug::*;

    #[test]
    fn need_and_traffic_convert_from_bool() {
        assert_eq!(ImageNeed::from(true), ImageNeed::Fetch);
        assert_eq!(ImageNeed::from(false), ImageNeed::Skip);
        assert_eq!(TrafficKind::from(true), TrafficKind::StateChanging);
        assert_eq!(TrafficKind::from(false), TrafficKind::Transactional);
        assert_eq!(ImageNeed::default(), ImageNeed::Skip);
    }

    #[test]
    fn core_failure_maps_every_variant() {
        use nerust_gui_viewmodel::debugger::StatusMessage as S;
        assert!(matches!(CoreFailure::no_core().thread_status(), S::NoCore));
        assert!(matches!(
            CoreFailure::no_core().register_status(),
            S::NoCore
        ));
        assert!(matches!(CoreFailure::no_core().write_status(), S::NoCore));
        assert!(matches!(CoreFailure::no_core().step_status(), S::NoCore));
        assert!(matches!(
            CoreFailure::no_core().pause_status(true),
            S::NoCore
        ));
        for failure in [
            CoreFailure::TransportGone,
            CoreFailure::Refused("boom".to_string()),
        ] {
            let status = failure.thread_status();
            assert!(matches!(status, S::ThreadFailed(_)));
        }
        assert!(matches!(
            CoreFailure::TransportGone.step_status(),
            S::ThreadFailed(_)
        ));
        assert!(matches!(
            CoreFailure::Refused("boom".to_string()).step_status(),
            S::StepFailed(_)
        ));
        assert!(matches!(
            CoreFailure::TransportGone.pause_status(true),
            S::ThreadFailed(_)
        ));
        assert!(matches!(
            CoreFailure::Refused("x".to_string()).pause_status(true),
            S::PauseFailed(_)
        ));
        assert!(matches!(
            CoreFailure::Refused("x".to_string()).pause_status(false),
            S::ResumeFailed(_)
        ));
        assert!(matches!(
            CoreFailure::Refused(String::new()).register_status(),
            S::RegisterRefused(_)
        ));
        assert!(matches!(
            CoreFailure::Refused("r".to_string()).register_status(),
            S::RegisterRefused(_)
        ));
        assert!(matches!(
            CoreFailure::Refused("w".to_string()).write_status(),
            S::WriteRefused(_)
        ));
        for error in [
            nerust_emu_thread::OperationError::WorkerUnavailable,
            nerust_emu_thread::OperationError::NoReply,
            nerust_emu_thread::OperationError::Reply("denied".to_string()),
        ] {
            let _ = CoreFailure::from(error).thread_status();
        }
    }

    #[test]
    fn drain_ctx_recovers_and_images_skip_resolves() {
        use crate::session::test_util::test_session;
        let session = test_session();
        let ctx = session.drain_ctx();
        let _guard = ctx.lock_txn();
        let mut flight = LiveImagesFlight::begin(None, ImageNeed::Skip, DEBUG_DRAIN_TIMEOUT);
        assert!(matches!(flight.poll(), LiveImagesPoll::Done(None)));
        let mut flight =
            LiveImagesFlight::begin(ctx.core.clone(), ImageNeed::Skip, DEBUG_DRAIN_TIMEOUT);
        assert!(matches!(flight.poll(), LiveImagesPoll::Done(_)));
    }
}
