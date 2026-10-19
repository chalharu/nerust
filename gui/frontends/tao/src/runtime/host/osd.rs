//! OSD notification orchestration: drain, modal flow, and in-frame
//! layer sync. Display state lives in `OsdManager`; this impl block
//! wires it to the session, dialogs, history, and renderer.

use std::time::Instant;

use nerust_gui_shell::session::access::PauseControl;
use nerust_render_traits::renderer::GpuRenderer;
use tao::event::WindowEvent;

use super::HostState;
use crate::aux::focus::{WindowHandler, aux_focus_step};
use crate::ports::WindowOpener;
use nerust_gui_shell::session::osd::ModalSource;
use nerust_gui_shell::session::osd::strip_rows;
use nerust_render_traits::ReservedStrip;
use nerust_render_traits::renderer::strip_pixels;

/// In-flight error dialog: blocking `show_error` runs on a thread.
/// `resume_after` resumes only what we paused, after the last ack.
pub(crate) struct ModalFlight {
    pub(super) done: std::sync::mpsc::Receiver<()>,
    pub(super) resume_after: bool,
}

/// Likely-Render gate from cheap state only. Pure so the Skip
/// side stays unit-testable without a renderer.
fn osd_sync_likely(
    generation: u64,
    last_seen: u64,
    visible: bool,
    status_on: bool,
    layer_live: bool,
) -> bool {
    generation != last_seen || visible || status_on || layer_live
}

impl HostState {
    fn notification_settings(&self) -> nerust_settings_core::model::local::NotificationSettings {
        self.session.settings_snapshot().local.notifications.clone()
    }

    fn notification_language(&self) -> nerust_settings_core::model::language::AppLanguage {
        self.session.settings_snapshot().shared.general.language
    }

    pub(crate) fn is_history_window(&self, id: tao::window::WindowId) -> bool {
        self.history_window
            .as_ref()
            .is_some_and(|handle| handle.tao_window_id() == id)
    }

    pub(crate) fn open_history(&mut self, opener: &dyn WindowOpener) {
        if self.history_window.is_some() {
            self.sync_history_window();
            return;
        }
        if let Some(handle) = opener.create_history(self.notification_language()) {
            self.history_window = Some(handle);
            // Fresh window needs full content regardless of cache.
            self.history_sync_key = None;
            self.sync_history_window();
        }
    }

    fn sync_history_window(&mut self) {
        let language = self.notification_language();
        let key = (self.session.notification_history_seq(), language);
        // Dirty gate: the sync runs every loop pass while open, but
        // row building is skipped unless content or language changed.
        if self.history_sync_key == Some(key) {
            return;
        }
        self.history_sync_key = Some(key);
        let lines = self.session.notification_history_lines(language);
        if let Some(handle) = self.history_window.as_mut() {
            handle.sync(lines, language);
        }
    }

    pub(crate) fn on_history_event(&mut self, event: WindowEvent) {
        if matches!(event, WindowEvent::Destroyed) {
            self.history_window = None;
            self.history_sync_key = None;
            return;
        }
        // Same aux-focus agreement as debugger/PPU: same-app moves
        // cancel a pending auto-pause; leaving anywhere arms it.
        let focused = matches!(event, WindowEvent::Focused(true));
        let resume = aux_focus_step(&mut self.focus, focused);
        if resume {
            self.resume_session();
        }
        if let Some(handle) = self.history_window.as_mut() {
            handle.on_event(event);
            if handle.close_requested() {
                self.history_window = None;
                return;
            }
            if handle.take_clear_request() {
                self.session.clear_notification_history();
                self.sync_history_window();
            }
        }
    }

    /// Fresh view holds no layer: re-push on the next sync instead
    /// of skipping on a stale fingerprint. Sole presenter reset.
    pub(crate) fn note_renderer_replaced(&mut self) {
        self.osd_presenter.invalidate();
    }

    /// Reserved edge strips in physical pixels. Static per settings
    /// (not per frame) so the renderer never churns surfaces.
    pub(crate) fn reserved_strip(&self) -> ReservedStrip {
        let settings = self.notification_settings();
        let client_h = self
            .window
            .as_ref()
            .map(|window| window.inner_size().height)
            .unwrap_or(0);
        let (top_rows, bottom_rows) = strip_rows(
            settings.area_mode,
            settings.position,
            settings.status_area_mode,
            settings.status_position,
            settings.status_enabled,
        );
        ReservedStrip {
            top: strip_pixels(client_h, top_rows, 1.0),
            bottom: strip_pixels(client_h, bottom_rows, 1.0),
        }
    }

    /// Present one error batch without blocking the loop.
    fn spawn_modal(&mut self, detail: String) {
        let dialogs = std::sync::Arc::clone(&self.dialogs);
        let language = self.notification_language();
        let resume_after = self.resume_after_modal;
        let (done_tx, done_rx) = std::sync::mpsc::channel();
        self.modal_flight = Some(ModalFlight {
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
            self.modal_flight = None;
            self.osd.modal_closed();
            if std::mem::replace(&mut self.resume_after_modal, false) {
                self.resume();
            }
        }
    }

    /// Reap a finished dialog, chain the next batch, and resume
    /// only once the queue drains (never between batches). A dead
    /// thread (disconnected sender) reaps as a failed ack: the queue
    /// must never wedge, or every later error would pile up unshown.
    fn reap_modal(&mut self) {
        let finished = self
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
            self.modal_flight = None;
            self.osd.modal_closed();
            if self.osd.modal_pending() == 0 {
                self.resume_after_modal = false;
                if resume_after {
                    self.resume();
                }
            }
        }
    }

    /// Present the next batch only when no dialog is showing.
    /// The blocking `show_error` runs on a short-lived thread
    /// (see `ModalFlight`): presenting from the loop would freeze
    /// every window and starve backends needing loop cooperation.
    fn present_modal(&mut self) {
        if self.modal_flight.is_none()
            && let Some(detail) = self.osd.take_modal_batch()
        {
            self.spawn_modal(detail);
        }
    }

    /// Drain shell notifications into display state, show aggregated
    /// error modals (auto-pausing first), and refresh the overlay.
    /// Cheap when idle (dirty-gated sync); safe to call every loop
    /// pass, including paused+unfocused where `on_update` stalls.
    pub(crate) fn poll_notifications(&mut self) {
        use nerust_gui_shell::session::osd::{PushInput, TransientPolicy};
        let now = Instant::now();
        let settings = self.notification_settings();
        let language = self.notification_language();
        let report = self.session.pump_osd(
            &mut self.osd,
            PushInput {
                policy: TransientPolicy::from(&settings),
                language,
                now,
            },
        );
        if report.modal_queued {
            // Auto-pause first, guarded by can_pause.
            // Only what we paused resumes, after the last ack.
            if self.session.can_pause() && !self.session.paused() {
                self.pause();
                self.resume_after_modal = true;
            }
        }
        self.reap_modal();
        self.present_modal();
        if report.changed {
            // Paint even idle: no main redraw is scheduled while
            // paused+unfocused, which is exactly when Paused lands.
            self.request_redraw();
        }
        if self.history_window.is_some() {
            self.sync_history_window();
        }
    }

    /// Rasterize the OSD layer on change and hand it to the renderer.
    /// Size changes re-rasterize; empty content clears the layer.
    /// Coordinates are renderer surface pixels (the content area
    /// below the menubar), not the full window.
    pub(crate) fn sync_osd_layer(
        &mut self,
        renderer: &mut dyn GpuRenderer,
        size: nerust_render_traits::SurfaceSize,
    ) {
        use nerust_gui_shell::session::osd::{OsdChrome, OsdSync, OsdSyncInput};
        let snapshot = self.session.settings_snapshot();
        let language = snapshot.shared.general.language;
        let chrome = OsdChrome::from(&snapshot.local.notifications);
        // Cheap pre-gate before any snapshot: no rows, no status,
        // and no live layer can only Skip.
        if !osd_sync_likely(
            self.osd.generation(),
            self.osd_presenter.last_seen(),
            self.osd.has_visible(),
            chrome.status_enabled,
            self.osd_layer_live,
        ) {
            return;
        }
        let metrics = self.session.metrics();
        let decision = self.osd_presenter.sync(
            &self.osd,
            OsdSyncInput {
                chrome,
                language,
                fps: metrics.emulation_fps,
                paused: metrics.paused,
                now: Instant::now(),
                width: size.width,
                height: size.height,
                ui_scale: 1.0,
                safe_insets: nerust_render_traits::renderer::SafeInsets::default(),
            },
        );
        match decision {
            OsdSync::Skip => {}
            OsdSync::Clear => {
                self.osd_layer_live = false;
                renderer.set_osd_layer(None);
            }
            OsdSync::Render(plan) => {
                self.osd_layer_live = true;
                let (width, height) = (plan.width, plan.height);
                let content = plan.into_content();
                renderer.set_osd_layer(self.osd_raster.rasterize(&content, width, height));
            }
        }
    }

    /// Wake the loop for OSD expiry while labels are visible.
    pub(crate) fn osd_wakeup(&self) -> Option<Instant> {
        self.osd.wakeup_deadline(Instant::now())
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
        // Same generation as last sync: only other signals pass.
        assert!(!osd_sync_likely(7, 7, false, false, false));
        assert!(osd_sync_likely(8, 7, false, false, false));
        assert!(osd_sync_likely(7, 7, true, false, false));
        assert!(osd_sync_likely(7, 7, false, true, false));
        assert!(osd_sync_likely(7, 7, false, false, true));
    }

    #[test]
    fn idle_sync_pushes_no_layer() {
        let support = TestHost::new();
        let mut host = support.host;
        let mut renderer = StubRenderer::default();
        // Clean peek, nothing showing, status off, no live layer:
        // the snapshot and the renderer stay untouched.
        host.sync_osd_layer(&mut renderer, surface());
        host.sync_osd_layer(&mut renderer, surface());
        assert!(renderer.layers.is_empty());
        assert!(!host.osd_layer_live);
    }

    #[test]
    fn status_toggle_renders_then_clears() {
        let support = TestHost::new();
        let mut host = support.host;
        let mut on = host.session.settings_snapshot().clone();
        on.local.notifications.status_enabled = true;
        host.apply_settings(on).expect("headless settings apply");
        let mut renderer = StubRenderer::default();
        host.sync_osd_layer(&mut renderer, surface());
        assert_eq!(renderer.layers, [true]);
        assert!(host.osd_layer_live);
        // Steady second frame reuses the layer: no re-push.
        host.sync_osd_layer(&mut renderer, surface());
        assert_eq!(renderer.layers, [true]);
        // Toggle off: one Clear lands, then idle skips again.
        let mut off = host.session.settings_snapshot().clone();
        off.local.notifications.status_enabled = false;
        host.apply_settings(off).expect("headless settings apply");
        host.sync_osd_layer(&mut renderer, surface());
        assert_eq!(renderer.layers, [true, false]);
        assert!(!host.osd_layer_live);
        host.sync_osd_layer(&mut renderer, surface());
        assert_eq!(renderer.layers, [true, false]);
    }
}
