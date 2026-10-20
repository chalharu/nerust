//! OSD notification orchestration: drain, modal flow, and in-frame
//! layer sync. Display state lives in `OsdManager`; this impl block
//! wires it to the session, dialogs, history, and renderer.

use std::time::Instant;

use nerust_gui_shell::session::access::PauseControl;
use nerust_render_traits::renderer::GpuRenderer;
use tao::event::WindowEvent;

use super::HostState;
use crate::aux::focus::{AuxWindow as _, pump_aux};
use crate::ports::WindowOpener;
use nerust_gui_shell::session::osd::ModalSource as _;
use nerust_render_traits::ReservedStrip;
use nerust_render_traits::renderer::strip_pixels;

/// In-flight error dialog: blocking `show_error` runs on a thread.
/// `resume_after` resumes only what we paused, after the last ack.
pub(crate) struct ModalFlight {
    pub(super) done: std::sync::mpsc::Receiver<()>,
    pub(super) resume_after: bool,
}

/// Shell-side OSD sync gate, calling only public presenter/manager
/// APIs (`last_seen`, `generation`, `has_visible`). The generation
/// comparison replaces the old peek-then-take flag; visibility, the
/// 2Hz status digits, and geometry (size/chrome moves boxes without
/// a generation bump) are the other pass signals. The live-layer memo
/// is deliberately not an input: a live but steady layer must Skip
/// without running the presenter.
/// (Ideal home is `OsdPresenter::should_sync` in the shell crate --
/// shell is out of scope here, so this free fn keeps the signal list
/// in one place instead of spreading raw scalars at the call site.
/// It calls only public shell APIs, matching exactly what the future
/// `OsdPresenter::should_sync(&self, &OsdManager, status_enabled,
/// geometry_changed) -> bool` would read; moving it is a pure
/// relocation with no behavior change.)
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

/// One settings snapshot per frame. Captured once per loop entry and
/// threaded by reference (cheap `Arc` clone): `poll_notifications`
/// and `render_osd_frame` share the frame instead of cloning the
/// snapshot twice per tick.
#[derive(Debug, Clone)]
pub(crate) struct FrameSnapshot(std::sync::Arc<FrameSnapshotData>);

#[derive(Debug)]
struct FrameSnapshotData {
    notifications: nerust_settings_core::model::local::NotificationSettings,
    language: nerust_settings_core::model::language::AppLanguage,
}

impl FrameSnapshot {
    fn capture(session: &nerust_gui_shell::session::SessionHandle) -> Self {
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
    pub(crate) fn frame_snapshot(&self) -> FrameSnapshot {
        FrameSnapshot::capture(&self.session)
    }

    pub(crate) fn is_history_window(&self, id: tao::window::WindowId) -> bool {
        self.osd
            .history_window
            .as_ref()
            .is_some_and(|handle| handle.tao_window_id() == id)
    }

    pub(crate) fn open_history(&mut self, opener: &dyn WindowOpener) {
        let frame = self.frame_snapshot();
        if self.osd.history_window.is_some() {
            self.sync_history_window(&frame);
            return;
        }
        if let Some(handle) = opener.create_history(frame.language()) {
            self.osd.history_window = Some(handle);
            // Fresh window needs full content regardless of cache.
            self.osd.history_sync_key.clear();
            self.sync_history_window(&frame);
        }
    }

    fn sync_history_window(&mut self, frame: &FrameSnapshot) {
        let language = frame.language();
        let key = (self.session.notification_history_seq(), language);
        // Dirty gate via Memo: the sync runs every loop pass while
        // open, but row building is skipped unless content or language
        // changed. Construction stays seq-gated by the caller.
        if !self.osd.history_sync_key.check(&key) {
            return;
        }
        let lines: std::sync::Arc<[nerust_gui_shell::session::osd::HistoryLine]> =
            self.session.notification_history_lines(language).into();
        if let Some(handle) = self.osd.history_window.as_mut() {
            handle.sync(lines, language);
        }
    }

    pub(crate) fn on_history_event(&mut self, event: WindowEvent) {
        // One capture for the whole event: sync and post-clear
        // re-sync share it.
        let frame = self.frame_snapshot();
        if matches!(event, WindowEvent::Destroyed) {
            self.osd.history_window = None;
            self.osd.history_sync_key.clear();
            return;
        }
        // Read-only viewer like settings: arrival only cancels a
        // pending pause and never resumes. A protocol-paused session
        // stays paused while browsing history.
        if let WindowEvent::Focused(focused) = event {
            self.on_aux_focus(crate::aux::focus::AuxKind::History, focused);
        }
        if let Some(handle) = self.osd.history_window.as_mut() {
            pump_aux(&mut *handle, event);
            if handle.close_requested() {
                self.osd.history_window = None;
                return;
            }
            if handle.take_clear_request() {
                self.session.clear_notification_history();
                self.sync_history_window(&frame);
            }
        }
    }

    /// Fresh view holds no layer: re-push on the next sync instead
    /// of skipping on a stale fingerprint. Sole presenter reset.
    pub(crate) fn note_renderer_replaced(&mut self) {
        self.osd.presenter.invalidate();
    }

    /// Reserved edge strips in physical pixels. Row policy lives in
    /// the shell (`OsdChrome::reserved_rows`); only pixel conversion
    /// stays here with the renderer. Static per settings (not per
    /// frame) so the renderer never churns surfaces.
    fn reserved_strip_with(&self, frame: &FrameSnapshot) -> ReservedStrip {
        use nerust_gui_shell::session::osd::OsdChrome;
        let chrome = OsdChrome::from(frame.notifications());
        let client_h = self
            .window
            .as_ref()
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
        let dialogs = std::sync::Arc::clone(&self.dialogs);
        let resume_after = self.osd.resume_after_modal;
        let (done_tx, done_rx) = std::sync::mpsc::channel();
        self.osd.modal_flight = Some(ModalFlight {
            done: done_rx,
            resume_after,
        });
        if std::thread::Builder::new()
            .name("nerust-error-dialog".to_string())
            .spawn(move || {
                dialogs.show_error(language, &detail);
                let _ = done_tx.send(());
            })
            .is_err()
        {
            // Thread spawn all but never fails; if it does, unblock
            // the queue and resume only what we paused, rather than
            // wedge the game behind an unseen error (history and logs
            // keep the record).
            log::error!("error dialog thread failed to spawn");
            self.osd.modal_flight = None;
            self.osd.manager.modal_closed();
            if std::mem::replace(&mut self.osd.resume_after_modal, false) {
                self.resume();
            }
        }
    }

    /// Reap a finished dialog, chain the next batch, and resume
    /// only once the queue drains (never between batches). A dead
    /// thread (disconnected sender) reaps as a failed ack: the queue
    /// must never wedge, or every later error would pile up unshown.
    fn reap_modal(&mut self) {
        let finished =
            self.osd
                .modal_flight
                .as_ref()
                .and_then(|flight| match flight.done.try_recv() {
                    Err(std::sync::mpsc::TryRecvError::Empty) => None,
                    done => Some((flight.resume_after, done.is_ok())),
                });
        if let Some((resume_after, normal)) = finished {
            if !normal {
                log::error!("error dialog thread died before ack");
            }
            self.osd.modal_flight = None;
            self.osd.modal_closed();
            if self.osd.modal_queue_len() == 0 {
                self.osd.resume_after_modal = false;
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

    /// Drain shell notifications into display state, show aggregated
    /// error modals (auto-pausing first), and refresh the overlay.
    /// Cheap when idle (dirty-gated sync); safe to call every loop
    /// pass, including paused+unfocused where `on_update` stalls.
    /// The frame is captured once by the caller and threaded through,
    /// so this pass clones the snapshot at most once.
    pub(crate) fn poll_notifications(&mut self, frame: &FrameSnapshot) {
        use nerust_gui_shell::session::osd::{PushInput, TransientPolicy};
        let now = Instant::now();
        let language = frame.language();
        let tick = self.session.tick_metrics();
        let report = self.session.pump_osd(
            &mut self.osd.manager,
            PushInput {
                policy: TransientPolicy::from(frame.notifications()),
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
                self.osd.resume_after_modal = true;
            }
        }
        self.reap_modal();
        self.present_modal(language);
        if report.changed {
            // Paint even idle: no main redraw is scheduled while
            // paused+unfocused, which is exactly when Paused lands.
            self.request_redraw();
        }
        if self.osd.history_window.is_some() {
            self.sync_history_window(frame);
        }
    }

    /// One capture per redraw: edge strips and the layer sync share
    /// the frame instead of cloning the snapshot twice.
    pub(crate) fn render_osd_frame(
        &mut self,
        renderer: &mut dyn GpuRenderer,
        size: nerust_render_traits::SurfaceSize,
        frame: &FrameSnapshot,
    ) {
        let strip = self.reserved_strip_with(frame);
        if self.osd.last_strip.check(&strip) {
            renderer.set_reserved_strip(strip);
        }
        self.sync_osd_layer(renderer, size, frame);
    }

    /// Rasterize the OSD layer on change and hand it to the renderer.
    /// Size changes re-rasterize; empty content clears the layer.
    /// Coordinates are renderer surface pixels (the content area
    /// below the menubar), not the full window.
    fn sync_osd_layer(
        &mut self,
        renderer: &mut dyn GpuRenderer,
        size: nerust_render_traits::SurfaceSize,
        frame: &FrameSnapshot,
    ) {
        use nerust_gui_shell::session::osd::{OsdChrome, OsdSync, OsdSyncInput};
        let language = frame.language();
        let chrome = OsdChrome::from(frame.notifications());
        // Cheap shell-side pre-gate before running the presenter:
        // generation, visibility, status liveness, and geometry
        // (size/chrome) mirror exactly the signals the presenter
        // reads (see `osd_should_sync`). Geometry rides a Memo so
        // steady state skips without a manual `!=` gate.
        let geometry = (size.width, size.height, chrome);
        // Peek (no record): the memo records only on a pass below,
        // so a skipped frame re-reports the move, exactly like the
        // old `!=` gate.
        let geometry_changed = self.osd.last_sync_geometry.get() != Some(&geometry);
        if !osd_should_sync(
            &self.osd.presenter,
            &self.osd.manager,
            chrome.status_enabled,
            geometry_changed,
        ) {
            return;
        }
        let tick = self.session.tick_metrics();
        let decision = self.osd.presenter.sync(
            &self.osd.manager,
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
        self.osd.last_sync_geometry.check(&geometry);
        match decision {
            OsdSync::Skip => {}
            OsdSync::Clear => {
                self.osd.layer_live = false;
                renderer.set_osd_layer(None);
            }
            OsdSync::Render(plan) => {
                self.osd.layer_live = true;
                let (width, height) = (plan.width, plan.height);
                let content = plan.into_content();
                renderer.set_osd_layer(self.osd.raster.rasterize(&content, width, height));
            }
        }
    }

    /// Wake the loop for OSD expiry while labels are visible.
    pub(crate) fn osd_wakeup(&self) -> Option<Instant> {
        self.osd.manager.wakeup_deadline(Instant::now())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::host::TestHost;
    use nerust_render_traits::{
        FrameBuffer, SurfaceSize,
        renderer::{GpuRenderer, OsdLayer, RenderResult, RendererError},
    };
    use raw_window_handle::{RawDisplayHandle, RawWindowHandle};

    #[derive(Debug, Default)]
    struct StubRenderer {
        layers: Vec<bool>,
    }

    impl GpuRenderer for StubRenderer {
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

        fn set_osd_layer(&mut self, layer: Option<OsdLayer>) {
            self.layers.push(layer.is_some());
        }

        fn update_render_profile(
            &mut self,
            _profile: &nerust_render_traits::VideoRenderProfile,
        ) -> Result<(), RendererError> {
            Ok(())
        }

        fn render(&mut self, _frame: &FrameBuffer) -> RenderResult {
            RenderResult::Skipped
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
        let chrome = OsdChrome::from(host.frame_snapshot().notifications());
        assert!(!chrome.status_enabled);
        // Idle: same generation, nothing visible, status off, no
        // geometry move -- the presenter stays untouched.
        assert!(!osd_should_sync(
            &host.osd.presenter,
            &host.osd.manager,
            false,
            false
        ));
        // Status liveness and geometry moves each pass alone (the
        // generation/visibility arms ride the same OR and are pinned
        // by the render tests below).
        assert!(osd_should_sync(
            &host.osd.presenter,
            &host.osd.manager,
            true,
            false
        ));
        assert!(osd_should_sync(
            &host.osd.presenter,
            &host.osd.manager,
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
        let frame = host.frame_snapshot();
        host.sync_osd_layer(&mut renderer, surface(), &frame);
        let frame = host.frame_snapshot();
        host.sync_osd_layer(&mut renderer, surface(), &frame);
        assert!(renderer.layers.is_empty());
        assert!(!host.osd.layer_live);
    }

    #[test]
    fn status_toggle_renders_then_clears() {
        let support = TestHost::new();
        let mut host = support.host;
        let mut on = host.session.settings_snapshot().clone();
        on.local.notifications.status_enabled = true;
        host.apply_settings(on).expect("headless settings apply");
        let mut renderer = StubRenderer::default();
        let frame = host.frame_snapshot();
        host.sync_osd_layer(&mut renderer, surface(), &frame);
        assert_eq!(renderer.layers, [true]);
        assert!(host.osd.layer_live);
        // Steady second frame reuses the layer: no re-push.
        let frame = host.frame_snapshot();
        host.sync_osd_layer(&mut renderer, surface(), &frame);
        assert_eq!(renderer.layers, [true]);
        // Toggle off: one Clear lands, then idle skips again.
        let mut off = host.session.settings_snapshot().clone();
        off.local.notifications.status_enabled = false;
        host.apply_settings(off).expect("headless settings apply");
        let frame = host.frame_snapshot();
        host.sync_osd_layer(&mut renderer, surface(), &frame);
        assert_eq!(renderer.layers, [true, false]);
        assert!(!host.osd.layer_live);
        let frame = host.frame_snapshot();
        host.sync_osd_layer(&mut renderer, surface(), &frame);
        assert_eq!(renderer.layers, [true, false]);
    }
}
