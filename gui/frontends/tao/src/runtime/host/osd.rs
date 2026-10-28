//! Native OSD effects; ApplicationHost owns notification and rendering state.

use std::time::Instant;

use nerust_frontend_host::{
    aux::OpenRequest,
    frame::{FrameCtx, FrameSnapshot},
    osd::{ModalPresentation, ModalPresenter},
};
use tao::event::WindowEvent;

use super::HostState;
use super::window::WakeNeed;
use crate::aux::focus::AuxKind;
use crate::ports::WindowOpener;

struct DialogPresenter<'a> {
    dialogs: &'a dyn crate::ports::DialogPorts,
    error_title: String,
}

impl ModalPresenter for DialogPresenter<'_> {
    fn present_modal(
        &self,
        presentation: ModalPresentation,
    ) -> Option<std::sync::mpsc::Receiver<()>> {
        Some(
            self.dialogs
                .show_error_async(self.error_title.clone(), presentation.detail),
        )
    }
}

impl HostState {
    pub(crate) fn open_history(&mut self, opener: &dyn WindowOpener) {
        let ctx = self.frame_ctx();
        let Some(request) = self.reserve_aux_window(AuxKind::History) else {
            return;
        };
        let id = match request {
            OpenRequest::Create(id) => id,
            OpenRequest::Present(_) => {
                self.sync_history_window(ctx.frame());
                return;
            }
            OpenRequest::Pending(_) => return,
        };
        let Some(session) = self.prepare_widget_session(id) else {
            return;
        };
        let opened = self.create_aux_window(
            id,
            opener,
            crate::ports::AuxOpenParams::Controls { session },
        );
        if opened {
            // Fresh window needs full content regardless of cache.
            self.core.invalidate_history();
            self.sync_history_window(ctx.frame());
        }
    }

    fn sync_history_window(&mut self, frame: &FrameSnapshot) {
        self.core.history_update(frame.language());
        if let Some(id) = self.aux.route_id(AuxKind::History) {
            self.aux.refresh_widget(id, &mut self.core);
        }
    }

    pub(crate) fn on_history_event(&mut self, event: WindowEvent) {
        let id = self.aux.route_id(AuxKind::History);
        if matches!(event, WindowEvent::Destroyed) {
            if let Some(id) = id {
                self.finish_widget_window(id, true);
            }
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
            self.finish_widget_window(outcome.id.expect("routed generation"), true);
            return;
        }
        if outcome.submitted {
            self.finish_widget_window(outcome.id.expect("routed generation"), false);
            let ctx = self.frame_ctx();
            self.sync_history_window(ctx.frame());
        }
    }

    /// A fresh destination has neither the layer nor the reserved strip.
    #[cfg(test)]
    pub(crate) fn poll_notifications(&mut self, ctx: &FrameCtx) {
        self.tick_osd_pump(ctx);
    }

    /// Execute only native effects after the application borrow ends.
    pub(super) fn tick_osd_pump(&mut self, ctx: &FrameCtx) -> WakeNeed {
        let turn = self.core.pump_osd(ctx, Instant::now());
        if let Some(presentation) = turn.modal {
            let token = presentation.token;
            let adapter = DialogPresenter {
                dialogs: self.dialogs.as_ref(),
                error_title: self.core.dialog_titles().error,
            };
            let presenter: &dyn ModalPresenter = &adapter;
            if let Some(receiver) = presenter.present_modal(presentation) {
                self.core.attach_osd_modal_receiver(token, receiver);
            }
        }
        if self.core.redraw_needed() {
            self.windows.request_redraw();
        }
        if self.aux.contains(AuxKind::History) {
            self.sync_history_window(ctx.frame());
        }
        // Receiver attachment adds a polling flight after the core's turn capture.
        WakeNeed {
            flight_pending: self.core.osd_flight_pending(),
            ..turn.wake
        }
    }

    /// Wake the loop for OSD expiry while labels are visible.
    #[cfg(test)]
    pub(crate) fn osd_wakeup(&self) -> Option<Instant> {
        self.core.osd_wakeup(Instant::now())
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
        use wakui::pause::PauseOwner;
        let mut host = TestHost::with_nes().host;
        host.load_test_rom();
        host.report_host_error("parked modal".into());
        host.note_main_departure();
        host.tick_focus_sync();
        let (ack, done) = std::sync::mpsc::channel();
        let ctx = host.frame_ctx();
        let presentation = host.core.pump_osd(&ctx, Instant::now()).modal.unwrap();
        assert!(host.attach_osd_modal_receiver(presentation.token, done));
        let wake = host.tick_osd_pump(&ctx);
        assert!(
            wake.flight_pending,
            "ACK must wake a paused, unfocused loop"
        );
        ack.send(()).unwrap();
        host.tick_osd_pump(&ctx);
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

    fn surface() -> (u32, u32) {
        (960, 720)
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
                .osd_layer()
                .is_due(host.osd_notifications().manager(), false, false)
        );
        // Status liveness and geometry moves each pass alone.
        assert!(
            host.osd_layer()
                .is_due(host.osd_notifications().manager(), true, false)
        );
        assert!(
            host.osd_layer()
                .is_due(host.osd_notifications().manager(), false, true)
        );
    }

    #[test]
    fn idle_sync_pushes_no_layer() {
        let support = TestHost::new();
        let mut host = support.host;
        let mut renderer = StubRenderer::default();
        // Nothing showing, status off: only the initial strip reservation
        // is delivered; subsequent frames leave the renderer untouched.
        let ctx = host.frame_ctx();
        host.prepare_osd_viewport(
            surface().0,
            surface().1,
            720,
            &ctx,
            std::time::Instant::now(),
        )
        .apply(&mut renderer);
        let ctx = host.frame_ctx();
        host.prepare_osd_viewport(
            surface().0,
            surface().1,
            720,
            &ctx,
            std::time::Instant::now(),
        )
        .apply(&mut renderer);
        assert!(renderer.layers.is_empty());
        assert_eq!(renderer.strips, [ReservedStrip::default()]);
        assert!(!host.osd_layer_live());
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
        host.prepare_osd_viewport(
            surface().0,
            surface().1,
            720,
            &ctx,
            std::time::Instant::now(),
        )
        .apply(&mut renderer);
        assert_eq!(renderer.layers, [true]);
        assert!(host.osd_layer_live());
        // Steady second frame reuses the layer: no re-push.
        let ctx = host.frame_ctx();
        host.prepare_osd_viewport(
            surface().0,
            surface().1,
            720,
            &ctx,
            std::time::Instant::now(),
        )
        .apply(&mut renderer);
        assert_eq!(renderer.layers, [true]);
        // Toggle off: one Clear lands, then idle skips again.
        let mut off = host.settings_snapshot().clone();
        off.local.notifications.status_enabled = false;
        host.apply_settings(off).expect("headless settings apply");
        let ctx = host.frame_ctx();
        host.prepare_osd_viewport(
            surface().0,
            surface().1,
            720,
            &ctx,
            std::time::Instant::now(),
        )
        .apply(&mut renderer);
        assert_eq!(renderer.layers, [true, false]);
        assert!(!host.osd_layer_live());
        let ctx = host.frame_ctx();
        host.prepare_osd_viewport(
            surface().0,
            surface().1,
            720,
            &ctx,
            std::time::Instant::now(),
        )
        .apply(&mut renderer);
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
        host.invalidate_osd();
        // Invalidated presenter re-runs the decision; with nothing
        // visible the outcome is still Skip, so no layer lands.
        let ctx = host.frame_ctx();
        host.prepare_osd_viewport(
            surface().0,
            surface().1,
            720,
            &ctx,
            std::time::Instant::now(),
        )
        .apply(&mut renderer);
        assert!(renderer.layers.is_empty());
        assert_eq!(renderer.strips, [ReservedStrip::default()]);
        assert!(!host.osd_layer_live());
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
    fn close_retires_owned_modal_receiver_queue_and_layer() {
        use nerust_frontend_host::osd::ModalAdvance;
        use wakui::pause::PauseOwner;
        let mut host = TestHost::new().host;
        host.set_pause_owner(PauseOwner::Settings, true);
        host.report_host_error("active on close".into());
        let ctx = host.frame_ctx();
        let presentation = host.core.pump_osd(&ctx, Instant::now()).modal.unwrap();
        let (sender, receiver) = std::sync::mpsc::channel();
        assert!(host.attach_osd_modal_receiver(presentation.token, receiver));
        host.report_host_error("queued on close".into());
        host.core.pump_osd(&ctx, Instant::now());
        assert!(host.osd_modal_pending());
        assert!(host.osd_flight_pending());
        host.prepare_close();
        assert!(sender.send(()).is_err());
        assert!(!host.osd_modal_pending());
        assert!(!host.osd_flight_pending());
        assert!(!host.osd_layer_live());
        assert_eq!(host.osd_pending(), 0);
        assert!(!host.pause_held());
        assert_eq!(
            host.acknowledge_osd_modal(presentation.token),
            ModalAdvance::Ignored
        );
        let mut renderer = StubRenderer::default();
        host.prepare_osd_viewport(
            surface().0,
            surface().1,
            720,
            &ctx,
            std::time::Instant::now(),
        )
        .apply(&mut renderer);
        assert!(renderer.strips.is_empty());
        assert!(renderer.layers.is_empty());
        host.prepare_close();
        assert!(host.core.pump_osd(&ctx, Instant::now()).modal.is_none());
    }

    #[test]
    fn frame_render_applies_strip_once() {
        let support = TestHost::new();
        let mut host = support.host;
        let mut renderer = StubRenderer::default();
        // First frame records the strip; the repeat reuses the memo.
        let ctx = host.frame_ctx();
        host.prepare_osd_viewport(
            surface().0,
            surface().1,
            720,
            &ctx,
            std::time::Instant::now(),
        )
        .apply(&mut renderer);
        let ctx = host.frame_ctx();
        host.prepare_osd_viewport(
            surface().0,
            surface().1,
            720,
            &ctx,
            std::time::Instant::now(),
        )
        .apply(&mut renderer);
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
        host.prepare_osd_viewport(
            surface().0,
            surface().1,
            720,
            &ctx,
            std::time::Instant::now(),
        )
        .apply(&mut old);
        assert_eq!(old.layers, [true]);
        assert_eq!(old.strips.len(), 1);

        host.invalidate_osd();
        assert!(!host.osd_layer_live());
        let mut fresh = StubRenderer::default();
        host.prepare_osd_viewport(
            surface().0,
            surface().1,
            720,
            &ctx,
            std::time::Instant::now(),
        )
        .apply(&mut fresh);
        assert_eq!(fresh.strips, old.strips);
        assert_eq!(fresh.layers, [true]);
        host.prepare_osd_viewport(
            surface().0,
            surface().1,
            720,
            &ctx,
            std::time::Instant::now(),
        )
        .apply(&mut fresh);
        assert_eq!(fresh.strips.len(), 1);
        assert_eq!(fresh.layers, [true]);
    }
}
