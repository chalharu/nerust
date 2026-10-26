//! Debugger and live-follow orchestration for `HostState`.
//! Drains, flights, aux-window management, and live PPU follow.

use nerust_frontend_host::application::DebugView;
use nerust_frontend_host::aux::OpenRequest;
use nerust_gui_shell::{debug::DebugDrainOutput, session::LiveImages};
use nerust_gui_viewmodel::debugger as debug_vm;

use super::host::{HostState, WakeNeed};
use crate::aux::focus::AuxKind;
use crate::ports::WindowOpener;

struct NativeDebugView<'a> {
    aux: &'a mut crate::aux::registry::AuxRegistry,
    applied: bool,
}

impl DebugView for NativeDebugView<'_> {
    fn apply_drain(&mut self, output: DebugDrainOutput) {
        self.aux.debug_apply(output.display, output.reselected);
        self.applied = true;
    }

    fn apply_live(&mut self, live: LiveImages, running: bool) {
        let Some(batch) = live else {
            return;
        };
        let images = batch
            .images
            .into_iter()
            .map(|image| debug_vm::DebugImageRgba {
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
}

impl HostState {
    /// Live follow is active: both aux windows open on a running loaded core.
    pub(crate) fn live_follow_active(&self) -> bool {
        self.core
            .live_follow_active(self.is_debug_open(), self.is_ppu_open())
    }

    /// Live-follow pulse entry (timer thread via `UserEvent::LiveTick`).
    pub(crate) fn live_tick(&mut self) {
        self.maybe_live_ppu();
    }

    /// Debug pump for the control-flow pass: live follow plus drain polling.
    pub(super) fn tick_debug_pump(&mut self) -> WakeNeed {
        let debug_open = self.is_debug_open();
        let ppu_open = self.is_ppu_open();
        let mut view = NativeDebugView {
            aux: &mut self.aux,
            applied: false,
        };
        let wake = self.core.pump_debug(debug_open, ppu_open, &mut view);
        if view.applied {
            let ctx = self.core.frame_ctx();
            self.sync_chrome(&ctx, false);
        }
        wake
    }

    /// Live PPU follow while the core runs: images-only refresh each interval.
    pub(crate) fn maybe_live_ppu(&mut self) {
        let debug_open = self.is_debug_open();
        let ppu_open = self.is_ppu_open();
        let mut view = NativeDebugView {
            aux: &mut self.aux,
            applied: false,
        };
        self.core.pump_live_debug(debug_open, ppu_open, &mut view);
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
        let Some((open_ppu, requests)) = self.aux.debug_take_requests() else {
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
        let snapshot = self.aux.debug_snapshot(ppu_open);
        // Contended bridge locks would wedge the UI thread:
        // all critical sections are short field copies.
        debug_assert!(snapshot.is_none() || self.aux.debug_locks_free());
        self.core.enqueue_debug(requests, snapshot);
        self.poll_debug_flight();
    }

    /// Poll the stored drain flight once; applies on completion.
    /// Returns whether a flight is still pending.
    pub(crate) fn poll_debug_flight(&mut self) -> bool {
        let mut view = NativeDebugView {
            aux: &mut self.aux,
            applied: false,
        };
        let pending = self.core.poll_debug(&mut view);
        if view.applied {
            let ctx = self.core.frame_ctx();
            self.sync_chrome(&ctx, false);
        }
        pending
    }

    /// Focus-driven chrome sync. Menu/title refresh always runs; no redraw.
    pub(crate) fn sync_debug_chrome(&mut self, ctx: &super::host::FrameCtx) {
        self.sync_chrome(ctx, false);
        if self.core.chrome_drain_due(self.is_debug_open()) {
            self.run_debug_drain(&[]);
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::runtime::host::TestHost;

    #[test]
    fn apply_drain_output_without_windows_syncs_chrome() {
        let support = TestHost::with_nes();
        let mut host = support.host;
        host.load_test_rom();
        let stub = crate::testkit::StubAux::new(crate::aux::focus::AuxKind::Debug);
        let applied = stub.applied_drain.clone();
        host.aux.insert(Box::new(stub));
        host.run_debug_drain(&[]);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
        while host.poll_debug_flight() {
            assert!(std::time::Instant::now() < deadline);
            std::thread::yield_now();
        }
        assert!(applied.get());
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
        use nerust_frontend_host::application::DebugView;
        super::NativeDebugView {
            aux: &mut host.aux,
            applied: false,
        }
        .apply_live(None, true);
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
