//! ForegroundController: window-ready retry owned by one module.
//!
//! Split out of the `AndroidFrontend` god object. Owns the foreground
//! resume flow (`try_resume`): it drives `Lifecycle` (retry state),
//! `RenderSync` (window/renderer), and `Storage` (last-media restore).
//! The frontend keeps wiring (event loop, session) and executes the
//! decisions only.

use std::time::Instant;

use nerust_gui_shell::session::access::PauseControl as _;
use winit::event_loop::{ActiveEventLoop, ControlFlow};

impl super::AndroidFrontend {
    pub(super) fn try_resume_foreground(&mut self, event_loop: &ActiveEventLoop) {
        if !self.is_resumed || !self.lifecycle.resume_pending() {
            return;
        }
        if let Some(retry_at) = self.lifecycle.retry_deadline() {
            if Instant::now() < retry_at {
                event_loop.set_control_flow(ControlFlow::WaitUntil(retry_at));
                return;
            }
            self.lifecycle.clear_retry_deadline();
        }
        log::info!(
            "try_resume_foreground: attempt {}",
            self.lifecycle.attempt_index()
        );
        match self.render_sync.ensure_window(
            event_loop,
            &self.session,
            &self.gpu_factory,
            &mut self.osd,
            &mut self.shell,
        ) {
            Ok(()) => self.finish_foreground_attempt(),
            Err(error) => {
                let attempt = self.lifecycle.attempt_index();
                log::warn!("try_resume_foreground: attempt {attempt} failed: {error}");
                if self.lifecycle.schedule_retry(self.is_resumed)
                    && let Some(retry_at) = self.lifecycle.retry_deadline()
                {
                    event_loop.set_control_flow(ControlFlow::WaitUntil(retry_at));
                }
            }
        }
    }

    /// Window is ready: restore lifecycle media, resume own auto-pause,
    /// and re-acquire audio. Error auto-pauses are never resumed here.
    fn finish_foreground_attempt(&mut self) {
        let attempt = self.lifecycle.attempt_index();
        self.lifecycle.record_success();
        if self.lifecycle.restore_pending() {
            self.restore_lifecycle_media(event_loop);
        }
        if self
            .lifecycle
            .resume_want_foreground_retry(self.dialogs.auto_paused())
        {
            self.resume();
            self.lifecycle.set_auto_paused(false);
            log::info!("try_resume_foreground: resumed session after lifecycle pause");
        }
        self.session.restart_audio();
        log::info!("try_resume_foreground: attempt {attempt} succeeded");
        self.render_sync.request_redraw(&mut self.shell);
    }

    /// Reload the last media reference after a lifecycle restore.
    /// Failures clear the hidden state so a missing ROM never wedges
    /// the next foreground; restore errors route to the error dialog.
    fn restore_lifecycle_media(&mut self, event_loop: &ActiveEventLoop) {
        log::info!(
            "try_resume_foreground: lifecycle_restore_pending=true; attempting to load last ROM and restore hidden lifecycle state"
        );
        match self.storage.load_last_media_reference() {
            Ok(Some(reference)) => self.restore_from_document_uri(event_loop, reference),
            Ok(None) => self.restore_from_library_id(event_loop),
            Err(error) => {
                log::warn!("try_resume_foreground: failed to read last media reference: {error}");
                self.session.report_external_error(
                    nerust_gui_shell::session::notification::FailureOrigin::Restore,
                    "Previous ROM reference is invalid; open it again".to_string(),
                );
                self.discard_restore_state();
            }
        }
    }

    /// Reconnect a document-tree URI; a missing document keeps the
    /// hidden snapshot until the user reconnects the tree.
    fn restore_from_document_uri(&mut self, event_loop: &ActiveEventLoop, reference: String) {
        match self.load_document_uri(event_loop, reference, true) {
            Ok(()) => {
                log::info!("try_resume_foreground: loaded last document URI for lifecycle restore");
            }
            Err(error) => {
                log::warn!(
                    "try_resume_foreground: failed to load last document URI for lifecycle restore: {error}"
                );
                self.session.report_external_error(
                    nerust_gui_shell::session::notification::FailureOrigin::Restore,
                    "Previous ROM is unavailable; open it again".to_string(),
                );
                log::info!(
                    "try_resume_foreground: preserving hidden state until the ROM URI is reconnected"
                );
            }
        }
    }

    /// Reload the last library ROM by id, with hidden-state restore.
    fn restore_from_library_id(&mut self, event_loop: &ActiveEventLoop) {
        match self.storage.load_last_rom_id() {
            Ok(Some(id)) => {
                if self.storage.rom_library.rom_path(&id).is_none() {
                    log::warn!("try_resume_foreground: stored ROM id={id} is missing");
                    self.discard_restore_state();
                } else {
                    match self.load_from_library_with_autosave(event_loop, &id, true) {
                        Ok(()) => {
                            log::info!(
                                "try_resume_foreground: loaded last ROM id={id} for lifecycle restore"
                            );
                            super::show_toast(
                                &self.app,
                                "Legacy ROM restored; use Open ROM to reconnect its document",
                            );
                        }
                        Err(error) => {
                            log::warn!(
                                "try_resume_foreground: failed to load last ROM id={id} for lifecycle restore: {error}"
                            );
                            self.discard_restore_state();
                        }
                    }
                }
            }
            Ok(None) => {
                log::info!("try_resume_foreground: no last ROM recorded");
                self.discard_restore_state();
            }
            Err(error) => {
                log::warn!("try_resume_foreground: failed to read last ROM id: {error}");
                self.discard_restore_state();
            }
        }
    }

    /// Drop pending restore flags plus the hidden lifecycle snapshot.
    fn discard_restore_state(&mut self) {
        self.session.clear_hidden_lifecycle_state();
        self.storage.clear_restore_pending();
        self.lifecycle.clear_restore_pending();
    }
}
