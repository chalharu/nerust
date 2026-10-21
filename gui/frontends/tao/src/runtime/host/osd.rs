//! OSD notification orchestration: drain, modal flow, and in-frame layer sync.
//! Display state lives in `OsdManager`; this impl block wires it to the session.

use std::time::Instant;

use nerust_render_traits::renderer::RenderPipeline;
use tao::event::WindowEvent;

use super::HostState;
use super::frame::{FrameCtx, FrameSnapshot};
use super::window::WakeNeed;
use crate::aux::focus::AuxKind;
use crate::ports::WindowOpener;
use nerust_render_traits::ReservedStrip;
use nerust_render_traits::renderer::strip_pixels;

/// In-flight error dialog: blocking `show_error` runs on a thread.
/// Pause ownership lives in the host arbiter for the entire queue lifetime.
pub(crate) struct ModalFlight {
    pub(super) done: std::sync::mpsc::Receiver<()>,
}

/// Shell-side OSD sync gate, calling only public presenter/manager APIs.
/// The live-layer memo is deliberately not an input: a live but steady layer must Skip.
fn osd_layer_due(
    presenter: &nerust_gui_shell::session::osd::OsdPresenter,
    manager: &nerust_gui_shell::session::osd::OsdManager,
    status_enabled: bool,
    geometry_changed: bool,
) -> bool {
    manager.generation() != presenter.last_seen()
        || manager.has_visible()
        || status_enabled
        || geometry_changed
}

impl HostState {
    pub(crate) fn open_history(&mut self, opener: &dyn WindowOpener) {
        let ctx = self.frame_ctx();
        if self.aux.contains(AuxKind::History) {
            self.sync_history_window(ctx.frame());
            return;
        }
        if let Some(handle) = opener.create_aux(
            AuxKind::History,
            crate::ports::AuxOpenParams::History {
                language: ctx.frame().language(),
            },
        ) {
            let id = self.aux.insert(handle);
            debug_assert_eq!(self.aux.route_id(AuxKind::History), Some(id));
            // Fresh window needs full content regardless of cache.
            self.osd.history.sync_key.clear();
            self.sync_history_window(ctx.frame());
        }
    }

    fn sync_history_window(&mut self, frame: &FrameSnapshot) {
        let language = frame.language();
        let key = (self.session.notification_history_seq(), language);
        // Versioned gate: row building is skipped unless content or language changed.
        if !self.osd.history.sync_key.check(&key) {
            return;
        }
        let lines: std::sync::Arc<[nerust_gui_shell::session::osd::HistoryLine]> =
            self.session.notification_history_lines(language).into();
        self.aux.history_sync(lines, language);
    }

    pub(crate) fn on_history_event(&mut self, event: WindowEvent) {
        if matches!(event, WindowEvent::Destroyed) {
            self.drop_history_window();
            return;
        }
        // Read-only viewer: shared aux-focus drive (arrival only
        // cancels a pending pause, never resumes).
        if let WindowEvent::Focused(focused) = event {
            let _ = self.drive_aux_focus(AuxKind::History, focused);
        }
        let domain = crate::aux::native::adapt_event(&event);
        let Some(outcome) = self.aux_on_event(AuxKind::History, domain) else {
            return;
        };
        if outcome.close {
            self.drop_history_window();
            return;
        }
        if outcome.cleared {
            self.session.clear_notification_history();
            let ctx = self.frame_ctx();
            self.sync_history_window(ctx.frame());
        }
    }

    /// A fresh destination has neither the layer nor the reserved strip.
    pub(crate) fn note_renderer_replaced(&mut self) {
        self.osd.layer.presenter.invalidate();
        self.osd.layer.last_strip.clear();
        self.osd.layer.last_sync_geometry.clear();
        self.osd.layer.layer_live = false;
    }

    /// Reserved edge strips in physical pixels; static per settings, not per frame.
    fn reserved_strip_with(&self, frame: &FrameSnapshot) -> ReservedStrip {
        use nerust_gui_shell::session::osd::OsdChrome;
        let chrome = OsdChrome::from(frame.notifications());
        let client_h = self.windows.client_height();
        let (top_rows, bottom_rows) = chrome.reserved_rows();
        ReservedStrip {
            top: strip_pixels(client_h, top_rows, 1.0),
            bottom: strip_pixels(client_h, bottom_rows, 1.0),
        }
    }

    /// Present one error batch without blocking the loop.
    fn spawn_modal(
        &mut self,
        detail: String,
        language: nerust_settings_core::model::language::AppLanguage,
    ) {
        let ack = self.dialogs.show_error_async(language, detail);
        self.osd.modal.flight = Some(ModalFlight { done: ack });
    }

    /// Reap a finished dialog, chain the next batch, and resume once the queue drains.
    fn reap_modal(&mut self) {
        let finished =
            self.osd
                .modal
                .flight
                .as_ref()
                .and_then(|flight| match flight.done.try_recv() {
                    Err(std::sync::mpsc::TryRecvError::Empty) => None,
                    done => Some(done.is_ok()),
                });
        if let Some(normal) = finished {
            if !normal {
                log::error!("error dialog thread died before ack");
            }
            self.osd.modal.flight = None;
            self.osd.modal_closed();
            if self.osd.modal_queue_len() == 0 {
                self.set_pause_owner(nerust_frontend_kernel::pause::PauseOwner::Modal, false);
                self.request_redraw();
            }
        }
    }

    /// Present one error batch without blocking the loop, via the
    /// [`OsdService`] modal ports (never the manager fields directly).
    fn present_modal(&mut self, language: nerust_settings_core::model::language::AppLanguage) {
        if let Some(detail) = self.osd.take_modal_batch() {
            self.spawn_modal(detail, language);
        }
    }

    /// Drain shell notifications into display state, show error modals, refresh the overlay.
    /// Cheap when idle; safe to call every loop pass.
    pub(crate) fn poll_notifications(&mut self, ctx: &FrameCtx) {
        use nerust_gui_shell::session::osd::{PushInput, TransientPolicy};
        let now = Instant::now();
        let language = ctx.frame().language();
        let tick = ctx.tick();
        let report = self.session.pump_osd(
            &mut self.osd.modal.manager,
            PushInput {
                policy: TransientPolicy::from(ctx.frame().notifications()),
                language,
                now,
                tick,
            },
        );
        if report.modal_queued {
            self.set_pause_owner(nerust_frontend_kernel::pause::PauseOwner::Modal, true);
        }
        self.reap_modal();
        self.present_modal(language);
        if report.changed {
            // Paint even idle: no main redraw is scheduled while
            // paused+unfocused, which is exactly when Paused lands.
            self.request_redraw();
        }
        if self.aux.contains(AuxKind::History) {
            self.sync_history_window(ctx.frame());
        }
    }

    /// OSD pump for the control-flow pass: drain plus expiry-driven wake.
    pub(super) fn tick_osd_pump(&mut self, ctx: &FrameCtx) -> WakeNeed {
        self.poll_notifications(ctx);
        // Post-pump the deadline implies a showing label: one call drives expiry and wait.
        let osd_deadline = self.osd_wakeup();
        if osd_deadline.is_some() {
            // Redraw while labels show so expiry applies even paused.
            self.request_redraw();
        }
        WakeNeed {
            osd_deadline,
            flight_pending: self.osd.modal.flight.is_some(),
            ..WakeNeed::default()
        }
    }

    /// One capture per redraw: edge strips and the layer sync share
    /// the context instead of cloning snapshots twice.
    pub(crate) fn render_osd_frame(
        &mut self,
        renderer: &mut dyn RenderPipeline,
        size: nerust_render_traits::SurfaceSize,
        ctx: &FrameCtx,
    ) {
        let strip = self.reserved_strip_with(ctx.frame());
        if self.osd.layer.last_strip.check(&strip) {
            renderer.set_reserved_strip(strip);
        }
        self.sync_osd_layer(renderer, size, ctx);
    }

    /// Rasterize the OSD layer on change and hand it to the renderer.
    /// Coordinates are renderer surface pixels, not the full window.
    fn sync_osd_layer(
        &mut self,
        renderer: &mut dyn RenderPipeline,
        size: nerust_render_traits::SurfaceSize,
        ctx: &FrameCtx,
    ) {
        use nerust_gui_shell::session::osd::{OsdChrome, OsdSync, OsdSyncInput};
        let language = ctx.frame().language();
        let chrome = OsdChrome::from(ctx.frame().notifications());
        // Cheap shell-side pre-gate before running the presenter.
        let geometry = (size.width, size.height, chrome);
        // Single-op memo: records and reports change atomically, so a
        // repeated geometry never re-arms the presenter below.
        let geometry_changed = self.osd.layer.last_sync_geometry.check(&geometry);
        if !osd_layer_due(
            &self.osd.layer.presenter,
            &self.osd.modal.manager,
            chrome.status_enabled,
            geometry_changed,
        ) {
            return;
        }
        let tick = ctx.tick();
        let decision = self.osd.layer.presenter.sync(
            &self.osd.modal.manager,
            OsdSyncInput {
                tick,
                chrome,
                language,
                now: Instant::now(),
                width: geometry.0,
                height: geometry.1,
                ui_scale: 1.0,
                safe_insets: nerust_gui_shell::session::osd::ViewInsets::default(),
            },
        );
        log::trace!(
            "osd sync geometry v{}",
            self.osd.layer.last_sync_geometry.version()
        );
        match decision {
            OsdSync::Skip => {}
            OsdSync::Clear => {
                self.osd.layer.layer_live = false;
                renderer.set_osd_layer(None);
            }
            OsdSync::Render(plan) => {
                self.osd.layer.layer_live = true;
                let (width, height) = (plan.width, plan.height);
                let content = plan.into_content();
                renderer.set_osd_layer(self.osd.layer.raster.rasterize(&content, width, height));
            }
        }
    }

    /// Wake the loop for OSD expiry while labels are visible.
    pub(crate) fn osd_wakeup(&self) -> Option<Instant> {
        self.osd.modal.manager.wakeup_deadline(Instant::now())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::host::TestHost;
    use nerust_render_traits::{
        FrameBuffer, SurfaceSize,
        renderer::{
            FrameSink, OsdLayer, OsdSink, ProfileSwitch, RenderResult, RendererError, StripSink,
            SurfaceLifecycle,
        },
    };
    use raw_window_handle::{RawDisplayHandle, RawWindowHandle};

    #[test]
    fn parked_modal_has_ack_backstop_and_releases_only_its_owner() {
        use nerust_frontend_kernel::pause::PauseOwner;
        let mut host = TestHost::with_nes().host;
        host.load_test_rom();
        host.set_pause_owner(PauseOwner::Modal, true);
        host.note_main_departure();
        host.tick_focus_sync();
        let (ack, done) = std::sync::mpsc::channel();
        host.osd.inject_modal_flight_for_test(ModalFlight { done });
        let ctx = host.frame_ctx();
        let wake = host.tick_osd_pump(&ctx);
        assert!(
            wake.flight_pending,
            "ACK must wake a paused, unfocused loop"
        );
        ack.send(()).unwrap();
        host.reap_modal();
        assert!(host.session().tick_metrics().paused, "focus still holds");
        assert!(!host.pause_policy.owns(PauseOwner::Modal));
        host.release_focus_pause();
        assert!(!host.session().tick_metrics().paused);
    }

    #[derive(Debug, Default)]
    struct StubRenderer {
        layers: Vec<bool>,
        strips: Vec<ReservedStrip>,
    }

    impl SurfaceLifecycle for StubRenderer {
        fn size(&self) -> SurfaceSize {
            SurfaceSize::new(960, 720)
        }

        fn attach(
            &mut self,
            _window_handle: RawWindowHandle,
            _display_handle: RawDisplayHandle,
            _size: SurfaceSize,
        ) -> Result<(), RendererError> {
            Ok(())
        }

        fn detach(&mut self) {}

        fn resize(&mut self, _size: SurfaceSize) {}
    }

    impl FrameSink for StubRenderer {
        fn render(&mut self, _frame: &FrameBuffer) -> RenderResult {
            RenderResult::Skipped
        }
    }

    impl OsdSink for StubRenderer {
        fn set_osd_layer(&mut self, layer: Option<OsdLayer>) {
            self.layers.push(layer.is_some());
        }
    }

    impl StripSink for StubRenderer {
        fn set_reserved_strip(&mut self, strip: ReservedStrip) {
            self.strips.push(strip);
        }
    }

    impl ProfileSwitch for StubRenderer {
        fn update_render_profile(
            &mut self,
            _profile: &nerust_render_traits::VideoRenderProfile,
        ) -> Result<(), RendererError> {
            Ok(())
        }
    }

    fn surface() -> SurfaceSize {
        SurfaceSize::new(960, 720)
    }

    #[test]
    fn sync_gate_covers_all_cheap_inputs() {
        use nerust_gui_shell::session::osd::OsdChrome;
        let support = TestHost::new();
        let mut host = support.host;
        let chrome = OsdChrome::from(host.frame_ctx().frame().notifications());
        assert!(!chrome.status_enabled);
        // Idle: same generation, nothing visible, status off, no
        // geometry move -- the presenter stays untouched.
        assert!(!osd_layer_due(
            &host.osd.layer.presenter,
            &host.osd.modal.manager,
            false,
            false
        ));
        // Status liveness and geometry moves each pass alone.
        assert!(osd_layer_due(
            &host.osd.layer.presenter,
            &host.osd.modal.manager,
            true,
            false
        ));
        assert!(osd_layer_due(
            &host.osd.layer.presenter,
            &host.osd.modal.manager,
            false,
            true
        ));
    }

    #[test]
    fn idle_sync_pushes_no_layer() {
        let support = TestHost::new();
        let mut host = support.host;
        let mut renderer = StubRenderer::default();
        // Clean peek, nothing showing, status off, no live layer:
        // the snapshot and the renderer stay untouched.
        let ctx = host.frame_ctx();
        host.sync_osd_layer(&mut renderer, surface(), &ctx);
        let ctx = host.frame_ctx();
        host.sync_osd_layer(&mut renderer, surface(), &ctx);
        assert!(renderer.layers.is_empty());
        assert!(!host.osd.layer.layer_live);
    }

    #[test]
    fn status_toggle_renders_then_clears() {
        let support = TestHost::new();
        let mut host = support.host;
        let mut on = host.session.settings_snapshot().clone();
        on.local.notifications.status_enabled = true;
        host.apply_settings(on).expect("headless settings apply");
        let mut renderer = StubRenderer::default();
        let ctx = host.frame_ctx();
        host.sync_osd_layer(&mut renderer, surface(), &ctx);
        assert_eq!(renderer.layers, [true]);
        assert!(host.osd.layer.layer_live);
        // Steady second frame reuses the layer: no re-push.
        let ctx = host.frame_ctx();
        host.sync_osd_layer(&mut renderer, surface(), &ctx);
        assert_eq!(renderer.layers, [true]);
        // Toggle off: one Clear lands, then idle skips again.
        let mut off = host.session.settings_snapshot().clone();
        off.local.notifications.status_enabled = false;
        host.apply_settings(off).expect("headless settings apply");
        let ctx = host.frame_ctx();
        host.sync_osd_layer(&mut renderer, surface(), &ctx);
        assert_eq!(renderer.layers, [true, false]);
        assert!(!host.osd.layer.layer_live);
        let ctx = host.frame_ctx();
        host.sync_osd_layer(&mut renderer, surface(), &ctx);
        assert_eq!(renderer.layers, [true, false]);
    }

    #[test]
    fn history_open_records_request_and_stays_closed() {
        use crate::aux::focus::AuxKind;
        use crate::test_opener::{FakeOpener, OpenerCall};
        let support = TestHost::new();
        let mut host = support.host;
        let opener = FakeOpener::failing("no display");
        host.open_history(&opener);
        host.open_history(&opener);
        assert_eq!(opener.calls(), [OpenerCall::History, OpenerCall::History]);
        assert!(!host.aux.contains(AuxKind::History));
        assert!(host.osd.history_key().is_none());
    }

    #[test]
    fn history_events_without_window_stay_total() {
        use tao::event::WindowEvent;
        let support = TestHost::new();
        let mut host = support.host;
        host.on_history_event(WindowEvent::Destroyed);
        host.on_history_event(WindowEvent::Focused(true));
        host.on_history_event(WindowEvent::Focused(false));
        assert!(host.osd.history_key().is_none());
        assert!(support.menu.updates().is_empty());
    }

    #[test]
    fn renderer_replace_repushes_without_layer() {
        let support = TestHost::new();
        let mut host = support.host;
        let mut renderer = StubRenderer::default();
        host.note_renderer_replaced();
        // Invalidated presenter re-runs the decision; with nothing
        // visible the outcome is still Skip, so no layer lands.
        let ctx = host.frame_ctx();
        host.sync_osd_layer(&mut renderer, surface(), &ctx);
        assert!(renderer.layers.is_empty());
        assert!(!host.osd.layer.layer_live);
    }

    #[test]
    fn idle_pump_reports_no_wake() {
        let support = TestHost::new();
        let mut host = support.host;
        assert!(host.osd_wakeup().is_none());
        let ctx = host.frame_ctx();
        let wake = host.tick_osd_pump(&ctx);
        assert!(wake.osd_deadline.is_none());
        assert!(wake.live_tick.is_none());
        assert!(!wake.flight_pending);
    }

    #[test]
    fn frame_render_applies_strip_once() {
        let support = TestHost::new();
        let mut host = support.host;
        let mut renderer = StubRenderer::default();
        // First frame records the strip; the repeat reuses the memo.
        let ctx = host.frame_ctx();
        host.render_osd_frame(&mut renderer, surface(), &ctx);
        let ctx = host.frame_ctx();
        host.render_osd_frame(&mut renderer, surface(), &ctx);
        assert!(renderer.layers.is_empty());
        assert_eq!(renderer.strips.len(), 1);
    }

    #[test]
    fn renderer_replacement_repushes_strip_and_visible_layer() {
        let support = TestHost::new();
        let mut host = support.host;
        let mut settings = host.session.settings_snapshot().clone();
        settings.local.notifications.status_enabled = true;
        host.apply_settings(settings)
            .expect("headless settings apply");
        let ctx = host.frame_ctx();
        let mut old = StubRenderer::default();
        host.render_osd_frame(&mut old, surface(), &ctx);
        assert_eq!(old.layers, [true]);
        assert_eq!(old.strips.len(), 1);

        host.note_renderer_replaced();
        assert!(!host.osd.layer.layer_live);
        let mut fresh = StubRenderer::default();
        host.render_osd_frame(&mut fresh, surface(), &ctx);
        assert_eq!(fresh.strips, old.strips);
        assert_eq!(fresh.layers, [true]);
        host.render_osd_frame(&mut fresh, surface(), &ctx);
        assert_eq!(fresh.strips.len(), 1);
        assert_eq!(fresh.layers, [true]);
    }
}
