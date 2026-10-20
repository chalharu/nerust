//! OSD notification orchestration: drain, modal flow, and in-frame layer sync.
//! Display state lives in `OsdManager`; this impl block wires it to the session.

use std::time::Instant;

use nerust_gui_shell::session::access::PauseControl;
use nerust_render_traits::renderer::GpuRenderer;
use tao::event::WindowEvent;

use super::{FrameCtx, HostState, WakeNeed};
use crate::aux::focus::AuxKind;
use crate::ports::WindowOpener;
use nerust_render_traits::ReservedStrip;
use nerust_render_traits::renderer::strip_pixels;

/// In-flight error dialog: blocking `show_error` runs on a thread.
/// `resume_after` resumes only what we paused, after the last ack.
pub(crate) struct ModalFlight {
    pub(super) done: std::sync::mpsc::Receiver<()>,
    pub(super) resume_after: bool,
}

/// Shell-side OSD sync gate, calling only public presenter/manager APIs.
/// The live-layer memo is deliberately not an input: a live but steady layer must Skip.
fn osd_should_sync(
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

/// One settings snapshot per entry point, threaded by reference.
#[derive(Debug, Clone)]
pub(crate) struct FrameSnapshot(std::sync::Arc<FrameSnapshotData>);

#[derive(Debug)]
struct FrameSnapshotData {
    notifications: nerust_settings_core::model::local::NotificationSettings,
    language: nerust_settings_core::model::language::AppLanguage,
}

impl FrameSnapshot {
    pub(super) fn capture(session: &nerust_gui_shell::session::SessionHandle) -> Self {
        let snapshot = session.settings_snapshot();
        Self(std::sync::Arc::new(FrameSnapshotData {
            notifications: snapshot.local.notifications.clone(),
            language: snapshot.shared.general.language,
        }))
    }

    pub(crate) fn notifications(
        &self,
    ) -> &nerust_settings_core::model::local::NotificationSettings {
        &self.0.notifications
    }

    pub(crate) fn language(&self) -> nerust_settings_core::model::language::AppLanguage {
        self.0.language
    }
}

impl HostState {
    pub(crate) fn open_history(&mut self, opener: &dyn WindowOpener) {
        let ctx = self.frame_ctx();
        if self.aux.contains(AuxKind::History) {
            self.sync_history_window(&ctx.frame);
            return;
        }
        if let Some(handle) = opener.create_history(ctx.frame.language()) {
            self.set_history_window(handle);
            // Fresh window needs full content regardless of cache.
            self.osd.history.sync_key.clear();
            self.sync_history_window(&ctx.frame);
        }
    }

    fn sync_history_window(&mut self, frame: &FrameSnapshot) {
        let language = frame.language();
        let key = (self.session.notification_history_seq(), language);
        // Dirty gate via Memo: row building is skipped unless content or language changed.
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
        // Read-only viewer: arrival only cancels a pending pause, never resumes.
        if let WindowEvent::Focused(focused) = event {
            self.on_aux_focus(AuxKind::History, focused);
        }
        let Some(outcome) = self.aux_on_event(AuxKind::History, event) else {
            return;
        };
        if outcome.close {
            self.drop_history_window();
            return;
        }
        if outcome.cleared {
            self.session.clear_notification_history();
            let ctx = self.frame_ctx();
            self.sync_history_window(&ctx.frame);
        }
    }

    /// Fresh view holds no layer: re-push on the next sync instead
    /// of skipping on a stale fingerprint. Sole presenter reset.
    pub(crate) fn note_renderer_replaced(&mut self) {
        self.osd.layer.presenter.invalidate();
    }

    /// Reserved edge strips in physical pixels; static per settings, not per frame.
    fn reserved_strip_with(&self, frame: &FrameSnapshot) -> ReservedStrip {
        use nerust_gui_shell::session::osd::OsdChrome;
        let chrome = OsdChrome::from(frame.notifications());
        let client_h = self
            .windows
            .live()
            .map(|window| window.inner_size().height)
            .unwrap_or(0);
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
        let resume_after = self.osd.modal.resume_after_modal;
        let ack = self.dialogs.show_error_async(language, detail);
        self.osd.modal.flight = Some(ModalFlight {
            done: ack,
            resume_after,
        });
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
                    done => Some((flight.resume_after, done.is_ok())),
                });
        if let Some((resume_after, normal)) = finished {
            if !normal {
                log::error!("error dialog thread died before ack");
            }
            self.osd.modal.flight = None;
            self.osd.modal_closed();
            if self.osd.modal_queue_len() == 0 {
                self.osd.modal.resume_after_modal = false;
                if resume_after {
                    self.resume();
                }
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
        let language = ctx.frame.language();
        let tick = ctx.tick;
        let report = self.session.pump_osd(
            &mut self.osd.modal.manager,
            PushInput {
                policy: TransientPolicy::from(ctx.frame.notifications()),
                language,
                now,
                tick,
            },
        );
        if report.modal_queued {
            // Auto-pause first, guarded by can_pause.
            // Only what we paused resumes, after the last ack.
            if tick.can_pause() && !tick.paused {
                self.pause();
                self.osd.modal.resume_after_modal = true;
            }
        }
        self.reap_modal();
        self.present_modal(language);
        if report.changed {
            // Paint even idle: no main redraw is scheduled while
            // paused+unfocused, which is exactly when Paused lands.
            self.request_redraw();
        }
        if self.aux.contains(AuxKind::History) {
            self.sync_history_window(&ctx.frame);
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
            ..WakeNeed::default()
        }
    }

    /// One capture per redraw: edge strips and the layer sync share
    /// the context instead of cloning snapshots twice.
    pub(crate) fn render_osd_frame(
        &mut self,
        renderer: &mut dyn GpuRenderer,
        size: nerust_render_traits::SurfaceSize,
        ctx: &FrameCtx,
    ) {
        let strip = self.reserved_strip_with(&ctx.frame);
        if self.osd.layer.last_strip.check(&strip) {
            renderer.set_reserved_strip(strip);
        }
        self.sync_osd_layer(renderer, size, ctx);
    }

    /// Rasterize the OSD layer on change and hand it to the renderer.
    /// Coordinates are renderer surface pixels, not the full window.
    fn sync_osd_layer(
        &mut self,
        renderer: &mut dyn GpuRenderer,
        size: nerust_render_traits::SurfaceSize,
        ctx: &FrameCtx,
    ) {
        use nerust_gui_shell::session::osd::{OsdChrome, OsdSync, OsdSyncInput};
        let language = ctx.frame.language();
        let chrome = OsdChrome::from(ctx.frame.notifications());
        // Cheap shell-side pre-gate before running the presenter.
        let geometry = (size.width, size.height, chrome);
        // Peek (no record): the memo records only on a pass below.
        let geometry_changed = self.osd.layer.last_sync_geometry.get() != Some(&geometry);
        if !osd_should_sync(
            &self.osd.layer.presenter,
            &self.osd.modal.manager,
            chrome.status_enabled,
            geometry_changed,
        ) {
            return;
        }
        let tick = ctx.tick;
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
                safe_insets: nerust_gui_shell::session::osd::UiInsets::default(),
            },
        );
        self.osd.layer.last_sync_geometry.check(&geometry);
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
            GpuRenderer, OsdLayer, OsdSink, Present, RenderResult, RendererError, StripSink,
        },
    };
    use raw_window_handle::{RawDisplayHandle, RawWindowHandle};

    #[derive(Debug, Default)]
    struct StubRenderer {
        layers: Vec<bool>,
    }

    impl Present for StubRenderer {
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

        fn render(&mut self, _frame: &FrameBuffer) -> RenderResult {
            RenderResult::Skipped
        }
    }

    impl OsdSink for StubRenderer {
        fn set_osd_layer(&mut self, layer: Option<OsdLayer>) {
            self.layers.push(layer.is_some());
        }
    }

    impl StripSink for StubRenderer {}

    impl GpuRenderer for StubRenderer {
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
        let host = support.host;
        let chrome = OsdChrome::from(host.frame_ctx().frame.notifications());
        assert!(!chrome.status_enabled);
        // Idle: same generation, nothing visible, status off, no
        // geometry move -- the presenter stays untouched.
        assert!(!osd_should_sync(
            &host.osd.layer.presenter,
            &host.osd.modal.manager,
            false,
            false
        ));
        // Status liveness and geometry moves each pass alone.
        assert!(osd_should_sync(
            &host.osd.layer.presenter,
            &host.osd.modal.manager,
            true,
            false
        ));
        assert!(osd_should_sync(
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
}
