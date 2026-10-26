//! Debugger and live-follow orchestration for `HostState`.
//! Drains, flights, aux-window management, and live PPU follow.

use std::{task::Poll, time::Instant};

use nerust_frontend_host::aux::OpenRequest;
use nerust_frontend_host::drain::{DrainEvent, poll_flight, prepare_pause_actions};
use nerust_gui_shell::{debug::DebugDrainOutput, session::LiveImages};
use nerust_gui_viewmodel::debugger as debug_vm;

use super::host::{HostState, WakeNeed};
use crate::aux::focus::AuxKind;
use crate::ports::WindowOpener;

impl HostState {
    pub(crate) fn take_debug_open_request(&mut self) -> bool {
        self.orchestrator_mut()
            .windows_mut()
            .take_debug_open_request()
    }

    pub(crate) fn take_ppu_open_request(&mut self) -> bool {
        self.orchestrator_mut()
            .windows_mut()
            .take_ppu_open_request()
    }

    /// Live follow is active: both aux windows open on a running loaded core.
    pub(crate) fn live_follow_active(&self) -> bool {
        self.is_debug_open() && self.is_ppu_open() && self.tick().loaded && !self.tick().paused
    }

    /// Live-follow pulse entry (timer thread via `UserEvent::LiveTick`).
    pub(crate) fn live_tick(&mut self) {
        self.maybe_live_ppu();
    }

    /// Debug pump for the control-flow pass: live follow plus drain polling.
    pub(super) fn tick_debug_pump(&mut self) -> WakeNeed {
        self.maybe_live_ppu();
        // Poll a pending drain flight (non-blocking); the backstop bounds the latency.
        let drain_pending = self.poll_debug_flight();
        // While the PPU follows a running core, wake for the next tick.
        let live_follow = self.live_follow_active();
        self.set_live_wanted(live_follow);
        WakeNeed {
            live_tick: live_follow.then(|| self.next_live_tick()).flatten(),
            flight_pending: drain_pending || self.live_flight_pending(),
            osd_deadline: None,
        }
    }

    /// Live PPU follow while the core runs: images-only refresh each interval.
    pub(crate) fn maybe_live_ppu(&mut self) {
        // Settle any in-flight fetch first (non-blocking).
        if self.poll_live_images() {
            return;
        }
        if !self.live_follow_active() {
            return;
        }
        if !self.live_due(Instant::now()) {
            return;
        }
        self.debug.live_flight = Some(self.debug_live_images_begin(true));
        self.poll_live_images();
    }

    fn poll_live_images(&mut self) -> bool {
        match poll_flight(&mut self.debug.live_flight) {
            Poll::Ready(Some(live)) => self.apply_live_images(live),
            Poll::Ready(None) => {}
            Poll::Pending => return true,
        }
        false
    }

    /// Patch a finished live fetch into the viewer: images land when newer.
    /// Bridge writes route through the registry narrow ports.
    fn apply_live_images(&mut self, live: LiveImages) {
        let Some(batch) = live else {
            return;
        };
        let running = !self.tick().paused;
        let images = batch
            .images
            .into_iter()
            .map(|image| nerust_gui_viewmodel::debugger::DebugImageRgba {
                label: image.label,
                width: image.width,
                height: image.height,
                rgba: image.rgba,
            })
            .collect();
        if self.aux.debug_apply_live(batch.generation, images, running) {
            self.aux.ppu_sync();
        }
    }

    /// Open the debugger (idempotent); the PPU viewer only for the toolbar button.
    /// Closing never resumes; `focus_ppu` opens-or-focuses the viewer.
    pub(crate) fn open_debug_windows(&mut self, opener: &dyn WindowOpener, focus_ppu: bool) {
        use nerust_gui_shell::debug::DebugAction;
        if let OpenRequest::Create(id) = self.aux.request_open(AuxKind::Debug) {
            let spaces = self.debug_spaces();
            if spaces.is_empty() {
                log::warn!("debugger: no spaces (no core)");
            }
            let mem_addr = spaces.first().map(|(_, _, start)| *start).unwrap_or(0);
            let position = self.debug_window_pos();
            let language = self.settings_snapshot().shared.general.language;
            let opened = opener
                .create_aux(
                    AuxKind::Debug,
                    crate::ports::AuxOpenParams::Debug {
                        display: debug_vm::DisplayCache::default(),
                        spaces,
                        mem_addr,
                        language,
                        position,
                    },
                )
                .is_some_and(|handle| self.aux.complete_open(id, handle).is_ok());
            if opened {
                self.run_debug_drain(&[DebugAction::Pause, DebugAction::Refresh]);
            } else {
                self.aux.cancel_open(id);
                log::error!("failed to open debugger window");
            }
        }
        if focus_ppu && !self.is_ppu_open() {
            if let Some(bridge) = self.debug_bridge_for_ppu() {
                let OpenRequest::Create(id) = self.aux.request_open(AuxKind::Ppu) else {
                    return;
                };
                let position = self.ppu_window_pos();
                let opened = opener
                    .create_aux(
                        AuxKind::Ppu,
                        crate::ports::AuxOpenParams::Ppu { bridge, position },
                    )
                    .is_some_and(|handle| self.aux.complete_open(id, handle).is_ok());
                if opened {
                    // Populate the image cache immediately: the viewer derives on Sync.
                    self.aux.ppu_sync();
                    // The viewer is open now, so re-read with images:
                    // the opening drain ran imageless.
                    self.run_debug_drain(&[]);
                } else {
                    self.aux.cancel_open(id);
                    log::error!("failed to open PPU viewer window");
                }
            }
        } else if focus_ppu {
            // Second and later presses: the viewer exists, so take
            // focus instead of no-op.
            self.aux.focus_kind(AuxKind::Ppu);
        }
    }

    /// Drain debugger requests and push one batched snapshot back.
    pub(crate) fn drain_debug_requests(&mut self) {
        if let Some(bridge) = self.debug_bridge_for_ppu() {
            bridge.bind_epoch(self.debug_epoch());
        }
        let Some((open_ppu, requests)) = self.take_debug_batch() else {
            return;
        };
        if open_ppu {
            self.request_ppu_open();
        }
        if requests.is_empty() {
            self.poll_debug_flight();
            return;
        }
        self.run_debug_drain(&requests);
    }

    /// Run one action batch through an async shell-drain flight.
    /// Sends only; the event loop polls, so the UI thread never blocks.
    pub(crate) fn run_debug_drain(&mut self, requests: &[nerust_gui_shell::debug::DebugAction]) {
        if let Some(bridge) = self.debug_bridge_for_ppu() {
            bridge.bind_epoch(self.debug_epoch());
        }
        let ppu_open = self.is_ppu_open();
        let snapshot = match self.aux.debug_snapshot(ppu_open) {
            Some(snapshot) => snapshot,
            None => {
                // No round-trip ran, so nothing could have moved:
                // still record truth for the focus gate below.
                self.set_last_drain_paused(self.tick().paused);
                return;
            }
        };
        // Contended bridge locks would wedge the UI thread:
        // all critical sections are short field copies.
        debug_assert!(self.aux.debug_locks_free());
        let epoch = self.debug_epoch();
        self.debug.drain.enqueue(epoch, requests, snapshot);
        self.poll_debug_flight();
    }

    fn start_debug_batch(&mut self, batch: super::host::debug::DrainBatch) {
        use nerust_gui_shell::debug::{DebugAction, DebugDrainInput};
        use nerust_gui_shell::session::access::PauseControl;
        let flight = batch.begin(|actions, snapshot| {
            prepare_pause_actions(actions, |action| {
                match action {
                    DebugAction::Pause => self.pause(),
                    DebugAction::Resume => self.resume(),
                    DebugAction::TogglePause => self.toggle_pause(),
                    _ => unreachable!("pause actions only"),
                }
                self.tick().loaded
            });
            let input = DebugDrainInput {
                space: snapshot.space,
                mem_addr: snapshot.mem_addr,
                dis_addr: snapshot.dis_addr,
                watch: snapshot.watch,
                freeze: snapshot.freeze,
                reg_target: snapshot.reg_target,
                prev_dump_rows: &snapshot.prev_rows,
                need_images: snapshot.ppu_open,
            };
            self.debug_drain_begin(actions, &input)
        });
        if let Some(flight) = flight {
            self.debug.drain.store(flight);
        }
    }

    /// Poll the stored drain flight once; applies on completion.
    /// Returns whether a flight is still pending.
    pub(crate) fn poll_debug_flight(&mut self) -> bool {
        loop {
            match self.debug.drain.poll() {
                DrainEvent::Idle => return false,
                DrainEvent::Pending => return true,
                DrainEvent::Start(batch) => self.start_debug_batch(batch),
                DrainEvent::Done(output, _) => self.apply_drain_output(*output),
            }
        }
    }

    /// Apply a finished drain: selection, running-sections merge, PPU follow.
    fn apply_drain_output(&mut self, output: DebugDrainOutput) {
        let reselected = output.reselected;
        let display = output.display;
        // Selection write plus paused-section restore live in the bridge narrow port.
        self.aux.debug_apply(display, reselected);
        // The drain is the single pause-truth agreement point: menu and title sync here.
        let ctx = self.frame_ctx();
        self.sync_chrome(&ctx, false);
        // Post-drain truth: focus flips compare against this to skip redundant round-trips.
        self.set_last_drain_paused(self.tick().paused);
    }

    /// Focus-driven chrome sync. Menu/title refresh always runs; no redraw.
    pub(crate) fn sync_debug_chrome(&mut self, ctx: &super::host::FrameCtx) {
        self.sync_chrome(ctx, false);
        if chrome_drain_due(
            self.is_debug_open(),
            self.last_drain_paused(),
            self.tick().paused,
        ) && !self.debug.drain_pending()
        {
            self.run_debug_drain(&[]);
        }
    }
}

fn chrome_drain_due(debug_open: bool, last_drain_paused: Option<bool>, paused: bool) -> bool {
    debug_open && last_drain_paused != Some(paused)
}

#[cfg(test)]
mod tests {
    use super::chrome_drain_due;
    use crate::runtime::host::TestHost;

    #[test]
    fn replacement_discards_waiting_old_epoch_batches_before_pause_or_write() {
        use crate::runtime::host::debug::DrainBatch;
        use nerust_gui_shell::debug::DebugAction;
        let support = TestHost::with_nes();
        let mut host = support.host;
        host.load_test_rom();
        assert!(matches!(
            host.session().debug_pause(),
            nerust_gui_viewmodel::debugger::StatusMessage::Paused
        ));
        let stub = crate::testkit::StubAux::new(crate::aux::focus::AuxKind::Debug);
        let applied = stub.applied_drain.clone();
        host.aux.insert(Box::new(stub));
        let epoch = host.session().debug_epoch();
        let mut snapshot = host.aux.debug_snapshot(false).unwrap();
        snapshot.space = Some(host.session().debug_spaces()[0].id);
        snapshot.reg_target = Some("a");
        snapshot.freeze = Some((0, 0xff));
        // Keep the first flight unpolled, with later mutations still queued.
        // Replacing the media now must drop both, without relying on timings.
        host.start_debug_batch(DrainBatch {
            epoch: epoch.clone(),
            actions: vec![DebugAction::StepInstr, DebugAction::RegisterWrite(0xff)],
            snapshot: snapshot.clone(),
        });
        host.debug.drain.push_back(DrainBatch {
            epoch: epoch.clone(),
            actions: vec![DebugAction::Pause, DebugAction::RegisterWrite(0xaa)],
            snapshot,
        });
        host.load_test_rom();
        assert!(!epoch.load(std::sync::atomic::Ordering::Acquire));
        assert!(!host.poll_debug_flight());
        assert!(!host.session().tick_metrics().paused);
        assert!(!host.debug.drain_pending());
        assert!(
            !applied.get(),
            "old output must never reach the replacement UI"
        );
        use nerust_gui_shell::session::access::PauseControl;
        host.pause();
        let current = host.session().debug_snapshot(
            None,
            0,
            None,
            nerust_gui_shell::debug::ImageNeed::Skip,
            None,
        );
        assert!(current.regs.contains(&("a", 0)));
    }

    #[test]
    fn queued_pause_waits_for_prior_real_worker_flight_and_chrome_cannot_replace_it() {
        use crate::runtime::host::debug::DrainBatch;
        use nerust_gui_shell::debug::DebugAction;
        use nerust_gui_shell::session::access::PauseControl;
        let support = TestHost::with_nes();
        let mut host = support.host;
        host.load_test_rom();
        host.pause();
        host.aux.insert(Box::new(crate::testkit::StubAux::new(
            crate::aux::focus::AuxKind::Debug,
        )));
        let epoch = host.session().debug_epoch();
        let mut snapshot = host.aux.debug_snapshot(false).unwrap();
        let space = host.session().debug_spaces()[0].id;
        snapshot.space = Some(space);
        snapshot.reg_target = Some("a");
        snapshot.freeze = Some((0, 0x5a));
        host.start_debug_batch(DrainBatch {
            epoch: epoch.clone(),
            actions: vec![DebugAction::StepInstr, DebugAction::RegisterWrite(0x45)],
            snapshot: snapshot.clone(),
        });
        host.debug.drain.push_back(DrainBatch {
            epoch,
            actions: vec![DebugAction::Resume],
            snapshot,
        });
        assert!(
            host.session().tick_metrics().paused,
            "resume has not started"
        );
        let ctx = host.frame_ctx();
        host.sync_debug_chrome(&ctx);
        assert_eq!(
            host.debug.drain.queued_len(),
            1,
            "chrome may not replace a pending drain"
        );
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
        while host.poll_debug_flight() {
            assert!(
                std::time::Instant::now() < deadline,
                "worker did not settle"
            );
            std::thread::yield_now();
        }
        assert!(!host.session().tick_metrics().paused);
        assert!(!host.debug.drain_pending());
        host.pause();
        assert_eq!(host.session().debug_read_byte(space, 0), Some(0x5a));
        let current = host.session().debug_snapshot(
            None,
            0,
            None,
            nerust_gui_shell::debug::ImageNeed::Skip,
            None,
        );
        assert!(current.regs.contains(&("a", 0x45)));
    }

    #[test]
    fn focus_chrome_drains_only_on_pause_transitions() {
        // Closed debugger: never worth a round-trip.
        assert!(!chrome_drain_due(false, None, false));
        assert!(!chrome_drain_due(false, Some(true), false));
        // First focus sync establishes the baseline.
        assert!(chrome_drain_due(true, None, true));
        assert!(chrome_drain_due(true, None, false));
        // Steady state: focus flips stay local-only.
        assert!(!chrome_drain_due(true, Some(true), true));
        assert!(!chrome_drain_due(true, Some(false), false));
        // A pause flip behind the toolbar's back (F9, auto-pause)
        // re-arms exactly one drain.
        assert!(chrome_drain_due(true, Some(true), false));
        assert!(chrome_drain_due(true, Some(false), true));
    }

    #[test]
    fn apply_drain_output_without_windows_syncs_chrome() {
        use nerust_gui_shell::debug::DebugDrainOutput;
        use nerust_gui_viewmodel::debugger::DisplayCache;
        let support = TestHost::new();
        let mut host = support.host;
        let output = DebugDrainOutput {
            display: DisplayCache::default(),
            reselected: Some((0x100, Some(0xAB))),
        };
        host.apply_drain_output(output);
        assert_eq!(
            host.last_drain_paused(),
            Some(host.session().tick_metrics().paused)
        );
        assert_eq!(support.menu.updates().len(), 1);
    }

    #[test]
    fn drain_and_live_stay_quiet_without_windows() {
        let support = TestHost::new();
        let mut host = support.host;
        assert!(!host.take_debug_open_request());
        assert!(!host.take_ppu_open_request());
        assert!(!host.live_follow_active());
        host.live_tick();
        assert!(!host.poll_debug_flight());
        // No debugger open: the drain records pause truth and stops.
        host.drain_debug_requests();
        host.run_debug_drain(&[]);
        assert_eq!(
            host.last_drain_paused(),
            Some(host.session().tick_metrics().paused)
        );
        assert!(support.menu.updates().is_empty());
    }

    #[test]
    fn tick_debug_pump_stays_idle_without_windows() {
        let support = TestHost::new();
        let mut host = support.host;
        host.live_tick();
        host.maybe_live_ppu();
        assert!(!host.live_follow_active());
        let wake = host.tick_debug_pump();
        assert!(wake.live_tick.is_none());
        assert!(!wake.flight_pending);
        assert!(wake.osd_deadline.is_none());
    }

    #[test]
    fn debugger_open_fails_closed_without_drain() {
        use crate::test_opener::{FakeOpener, OpenerCall};
        let support = TestHost::new();
        let mut host = support.host;
        let opener = FakeOpener::failing("no display");
        host.open_debug_windows(&opener, false);
        assert_eq!(opener.calls(), [OpenerCall::Debug]);
        assert!(!host.is_debug_open());
        // Failed open runs no drain, so the focus gate stays unset.
        assert_eq!(host.last_drain_paused(), None);
    }

    #[test]
    fn pending_debugger_open_skips_native_creation_and_drain() {
        use super::{AuxKind, OpenRequest};
        use crate::test_opener::FakeOpener;
        let mut host = TestHost::new().host;
        let OpenRequest::Create(id) = host.aux.request_open(AuxKind::Debug) else {
            panic!("expected reservation");
        };
        let opener = FakeOpener::failing("no display");
        host.open_debug_windows(&opener, true);
        assert!(opener.calls().is_empty());
        assert_eq!(host.aux.route_id(AuxKind::Debug), Some(id));
        assert!(!host.is_ppu_open());
        assert_eq!(host.last_drain_paused(), None);
    }

    #[test]
    fn viewer_open_without_debugger_needs_no_bridge() {
        use crate::test_opener::{FakeOpener, OpenerCall};
        let support = TestHost::new();
        let mut host = support.host;
        let opener = FakeOpener::failing("no display");
        host.open_debug_windows(&opener, true);
        assert_eq!(opener.calls(), [OpenerCall::Debug]);
        assert!(!host.is_ppu_open());
    }

    #[test]
    fn live_images_without_payload_stay_total() {
        let support = TestHost::new();
        let mut host = support.host;
        host.apply_live_images(None);
        assert!(!host.live_follow_active());
    }

    #[test]
    fn sync_chrome_without_debugger_skips_drain() {
        let support = TestHost::new();
        let mut host = support.host;
        let ctx = host.frame_ctx();
        host.sync_debug_chrome(&ctx);
        assert_eq!(support.menu.updates().len(), 1);
        // No debugger open: no drain runs, so the gate stays unset.
        assert_eq!(host.last_drain_paused(), None);
    }

    use crate::testkit::StubAux;

    fn batch_stub(open_ppu: bool, batch: Vec<nerust_gui_shell::debug::DebugAction>) -> StubAux {
        let mut stub = StubAux::new(crate::aux::focus::AuxKind::Debug);
        stub.drain_open = open_ppu;
        stub.debug_batch = batch;
        stub
    }

    #[test]
    fn empty_batch_with_ppu_flag_arms_viewer_open() {
        let support = TestHost::new();
        let mut host = support.host;
        host.aux.insert(Box::new(batch_stub(true, Vec::new())));
        host.drain_debug_requests();
        assert!(host.take_ppu_open_request());
        assert!(support.menu.updates().is_empty());
    }

    #[test]
    fn nonempty_batch_without_snapshot_records_truth() {
        use nerust_gui_shell::debug::DebugAction;
        let support = TestHost::new();
        let mut host = support.host;
        host.aux
            .insert(Box::new(batch_stub(false, vec![DebugAction::Refresh])));
        // Stub snapshot stays None, so the round trip never runs.
        assert!(host.aux.debug_locks_free());
        host.drain_debug_requests();
        assert_eq!(
            host.last_drain_paused(),
            Some(host.session().tick_metrics().paused)
        );
        assert!(!host.take_ppu_open_request());
    }
}
