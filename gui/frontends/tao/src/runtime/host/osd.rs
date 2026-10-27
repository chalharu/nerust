//! OSD notification orchestration: drain, modal flow, and in-frame layer sync.
//! Display state lives in `OsdManager`; this impl block wires it to the session.

use std::time::Instant;

use nerust_frontend_host::{
    aux::OpenRequest,
    frame::{FrameCtx, FrameSnapshot},
    osd::{ModalAdvance, ModalPresentation},
};
use nerust_render_traits::renderer::RenderPipeline;
use tao::event::WindowEvent;

use super::HostState;
use super::window::WakeNeed;
use crate::aux::focus::AuxKind;
use crate::ports::WindowOpener;

impl HostState {
    pub(crate) fn open_history(&mut self, opener: &dyn WindowOpener) {
        let ctx = self.frame_ctx();
        let id = match self.aux.request_open(AuxKind::History) {
            OpenRequest::Create(id) => id,
            OpenRequest::Present(_) => {
                self.sync_history_window(ctx.frame());
                return;
            }
            OpenRequest::Pending(_) => return,
        };
        let opened = opener
            .create_aux(
                AuxKind::History,
                crate::ports::AuxOpenParams::History {
                    language: ctx.frame().language(),
                },
            )
            .is_some_and(|handle| self.aux.complete_open(id, handle).is_ok());
        if opened {
            // Fresh window needs full content regardless of cache.
            self.core.invalidate_history();
            self.sync_history_window(ctx.frame());
        } else {
            self.aux.cancel_open(id);
        }
    }

    fn sync_history_window(&mut self, frame: &FrameSnapshot) {
        let language = frame.language();
        let Some(lines) = self.core.history_update(language) else {
            return;
        };
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
            self.on_focus_event(nerust_frontend_host::focus::FocusEvent::Aux(
                AuxKind::History,
                focused,
            ));
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
            self.core.clear_history();
            let ctx = self.frame_ctx();
            self.sync_history_window(ctx.frame());
        }
    }

    /// A fresh destination has neither the layer nor the reserved strip.
    pub(crate) fn note_renderer_replaced(&mut self) {
        self.osd.layer.invalidate();
    }

    /// Present one error batch without blocking the loop.
    fn spawn_modal(
        &mut self,
        presentation: ModalPresentation,
        language: nerust_settings_core::model::language::AppLanguage,
    ) {
        let ack = self.dialogs.show_error_async(language, presentation.detail);
        self.osd
            .modal
            .attach_modal_receiver(presentation.token, ack);
    }

    /// Reap a finished dialog, chain the next batch, and resume once the queue drains.
    fn reap_modal(&mut self, language: nerust_settings_core::model::language::AppLanguage) {
        if let Some(ModalAdvance::Present(presentation)) =
            self.core.poll_modal_ack(&mut self.osd.modal)
        {
            self.spawn_modal(presentation, language);
        }
    }

    /// Present one error batch without blocking the loop, via the
    /// [`OsdService`] modal ports (never the manager fields directly).
    fn present_modal(&mut self, language: nerust_settings_core::model::language::AppLanguage) {
        if let Some(presentation) = self.osd.modal.take_modal() {
            self.spawn_modal(presentation, language);
        }
    }

    /// Drain shell notifications into display state, show error modals, refresh the overlay.
    /// Cheap when idle; safe to call every loop pass.
    pub(crate) fn poll_notifications(&mut self, ctx: &FrameCtx) {
        let language = ctx.frame().language();
        let report = self
            .core
            .pump_notifications(&mut self.osd.modal, ctx, Instant::now());
        self.reap_modal(language);
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
            flight_pending: self.osd.modal.flight_pending(),
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
        self.osd.layer.render(
            self.osd.modal.manager(),
            renderer,
            size,
            self.windows.client_height(),
            ctx,
            Instant::now(),
        );
    }

    /// Rasterize the OSD layer on change and hand it to the renderer.
    /// Coordinates are renderer surface pixels, not the full window.
    #[cfg(test)]
    fn sync_osd_layer(
        &mut self,
        renderer: &mut dyn RenderPipeline,
        size: nerust_render_traits::SurfaceSize,
        ctx: &FrameCtx,
    ) {
        self.osd.layer.sync_layer(
            self.osd.modal.manager(),
            renderer,
            size,
            ctx,
            Instant::now(),
        );
    }

    /// Wake the loop for OSD expiry while labels are visible.
    pub(crate) fn osd_wakeup(&self) -> Option<Instant> {
        self.osd.modal.wakeup_deadline(Instant::now())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::host::TestHost;
    use nerust_render_traits::{
        FrameBuffer, ReservedStrip, SurfaceSize,
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
        host.report_host_error("parked modal".into());
        host.note_main_departure();
        host.tick_focus_sync();
        let (ack, done) = std::sync::mpsc::channel();
        let ctx = host.frame_ctx();
        host.core
            .pump_notifications(&mut host.osd.modal, &ctx, Instant::now());
        let presentation = host.osd.modal.take_modal().unwrap();
        assert!(
            host.osd
                .modal
                .attach_modal_receiver(presentation.token, done)
        );
        let wake = host.tick_osd_pump(&ctx);
        assert!(
            wake.flight_pending,
            "ACK must wake a paused, unfocused loop"
        );
        ack.send(()).unwrap();
        host.reap_modal(ctx.frame().language());
        assert!(host.session().tick_metrics().paused, "focus still holds");
        assert!(!host.owns_pause(PauseOwner::Modal));
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
        let support = TestHost::new();
        let mut host = support.host;
        let chrome = *host.frame_ctx().frame().chrome();
        assert!(!chrome.status_enabled);
        // Idle: same generation, nothing visible, status off, no
        // geometry move -- the presenter stays untouched.
        assert!(
            !host
                .osd
                .layer
                .is_due(host.osd.modal.manager(), false, false)
        );
        // Status liveness and geometry moves each pass alone.
        assert!(host.osd.layer.is_due(host.osd.modal.manager(), true, false));
        assert!(host.osd.layer.is_due(host.osd.modal.manager(), false, true));
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
        assert!(!host.osd.layer.layer_live());
    }

    #[test]
    fn status_toggle_renders_then_clears() {
        let support = TestHost::new();
        let mut host = support.host;
        let mut on = host.settings_snapshot().clone();
        on.local.notifications.status_enabled = true;
        host.apply_settings(on).expect("headless settings apply");
        let mut renderer = StubRenderer::default();
        let ctx = host.frame_ctx();
        host.sync_osd_layer(&mut renderer, surface(), &ctx);
        assert_eq!(renderer.layers, [true]);
        assert!(host.osd.layer.layer_live());
        // Steady second frame reuses the layer: no re-push.
        let ctx = host.frame_ctx();
        host.sync_osd_layer(&mut renderer, surface(), &ctx);
        assert_eq!(renderer.layers, [true]);
        // Toggle off: one Clear lands, then idle skips again.
        let mut off = host.settings_snapshot().clone();
        off.local.notifications.status_enabled = false;
        host.apply_settings(off).expect("headless settings apply");
        let ctx = host.frame_ctx();
        host.sync_osd_layer(&mut renderer, surface(), &ctx);
        assert_eq!(renderer.layers, [true, false]);
        assert!(!host.osd.layer.layer_live());
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
        assert!(host.history_key().is_none());
    }

    #[test]
    fn pending_history_open_skips_creation_and_content_cache_updates() {
        use crate::test_opener::FakeOpener;
        let mut host = TestHost::new().host;
        let OpenRequest::Create(id) = host.aux.request_open(AuxKind::History) else {
            panic!("expected reservation");
        };
        let opener = FakeOpener::failing("no display");
        host.open_history(&opener);
        assert!(opener.calls().is_empty());
        assert_eq!(host.aux.route_id(AuxKind::History), Some(id));
        assert!(host.history_key().is_none());
    }

    #[test]
    fn history_events_without_window_stay_total() {
        use tao::event::WindowEvent;
        let support = TestHost::new();
        let mut host = support.host;
        host.on_history_event(WindowEvent::Destroyed);
        host.on_history_event(WindowEvent::Focused(true));
        host.on_history_event(WindowEvent::Focused(false));
        assert!(host.history_key().is_none());
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
        assert!(!host.osd.layer.layer_live());
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
        let mut settings = host.settings_snapshot().clone();
        settings.local.notifications.status_enabled = true;
        host.apply_settings(settings)
            .expect("headless settings apply");
        let ctx = host.frame_ctx();
        let mut old = StubRenderer::default();
        host.render_osd_frame(&mut old, surface(), &ctx);
        assert_eq!(old.layers, [true]);
        assert_eq!(old.strips.len(), 1);

        host.note_renderer_replaced();
        assert!(!host.osd.layer.layer_live());
        let mut fresh = StubRenderer::default();
        host.render_osd_frame(&mut fresh, surface(), &ctx);
        assert_eq!(fresh.strips, old.strips);
        assert_eq!(fresh.layers, [true]);
        host.render_osd_frame(&mut fresh, surface(), &ctx);
        assert_eq!(fresh.strips.len(), 1);
        assert_eq!(fresh.layers, [true]);
    }
}
