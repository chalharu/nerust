//! OSD notification orchestration: drain, modal flow, and in-frame
//! layer sync. Display state lives in `OsdManager`; this impl block
//! wires it to the session, dialogs, history, and renderer.

use std::time::Instant;

use nerust_gui_shell::session::access::PauseControl;
use nerust_render_traits::renderer::GpuRenderer;
use tao::event::WindowEvent;

use super::HostState;
use crate::ports::WindowOpener;
use nerust_gui_shell::session::osd::OsdManager;

/// In-flight error dialog: blocking `show_error` runs on a thread.
/// `resume_after` resumes only what we paused, after the last ack.
pub(crate) struct ModalFlight {
    pub(super) done: std::sync::mpsc::Receiver<()>,
    pub(super) resume_after: bool,
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
            .is_some_and(|handle| handle.shell.window.id() == id)
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
        if let Some(handle) = self.history_window.as_mut() {
            crate::aux::shell::AuxWindow::pump_window_event(handle, event);
            if handle
                .shell
                .should_close
                .load(std::sync::atomic::Ordering::Acquire)
            {
                self.history_window = None;
                return;
            }
            if handle.take_clear_request() {
                self.session.clear_notification_history();
                self.sync_history_window();
            }
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
        let now = Instant::now();
        let drained = self.session.drain_notifications();
        if !drained.is_empty() {
            let settings = self.notification_settings();
            let language = self.notification_language();
            self.osd.push_all(drained, &settings, language, now);
        }
        self.osd.poll(now);
        let modal = self.osd.take_modal();
        if !modal.is_empty() {
            // Auto-pause first, guarded by can_pause.
            // Only what we paused resumes, after the last ack.
            if self.session.can_pause() && !self.session.paused() {
                self.pause();
                self.resume_after_modal = true;
            }
            let detail = modal
                .iter()
                .map(|line| format!("• {line}"))
                .collect::<Vec<_>>()
                .join("\n");
            self.osd.queue_modal(detail);
        }
        self.reap_modal();
        self.present_modal();
        if self.osd.dirty() {
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
        let (w, h) = (size.width, size.height);
        if w == 0 || h == 0 {
            if self.osd_layer_active {
                renderer.set_osd_layer(None);
                self.osd_layer_active = false;
            }
            return;
        }
        let dirty = self.osd.take_dirty();
        if !dirty && self.osd_layer_size == Some((w, h)) {
            return;
        }
        self.osd_layer_size = Some((w, h));
        let settings = self.notification_settings();
        let language = self.notification_language();
        let metrics = self.session.metrics();
        let status = settings.status_enabled.then(|| {
            let text = OsdManager::status_text(metrics.emulation_fps, metrics.paused, language);
            nerust_render_osd::OsdRow {
                tag: String::new(),
                text,
                rgb: (200, 200, 200),
            }
        });
        let rows = self
            .osd
            .rows()
            .iter()
            .map(|row| nerust_render_osd::OsdRow {
                tag: row.tag.to_string(),
                text: row.text.clone(),
                rgb: row.rgb,
            })
            .collect();
        let content = nerust_render_osd::OsdContent {
            rows,
            status,
            position: settings.position,
            status_position: settings.status_position,
        };
        if content.rows.is_empty() && content.status.is_none() {
            if self.osd_layer_active {
                renderer.set_osd_layer(None);
                self.osd_layer_active = false;
            }
            return;
        }
        let layer = self.osd_raster.rasterize(&content, w, h);
        self.osd_layer_active = layer.is_some();
        renderer.set_osd_layer(layer);
    }

    /// Wake the loop for OSD expiry while labels are visible.
    pub(crate) fn osd_wakeup(&self) -> Option<Instant> {
        self.osd.next_deadline()
    }
}
