//! ForegroundController: window-ready retry owned by one module.
//! Drives `Lifecycle`, `RenderSync`, and `Storage`; the frontend executes.

use std::time::Instant;

use nerust_gui_shell::session::access::PauseControl as _;
use winit::event_loop::{ActiveEventLoop, ControlFlow};

use super::picker;

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
            Ok(()) => self.finish_foreground_attempt(event_loop),
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
    fn finish_foreground_attempt(&mut self, event_loop: &ActiveEventLoop) {
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
    /// Failures clear hidden state; restore errors route to the error dialog.
    fn restore_lifecycle_media(&mut self, event_loop: &ActiveEventLoop) {
        log::info!(
            "try_resume_foreground: lifecycle_restore_pending=true; attempting to load last ROM and restore hidden lifecycle state"
        );
        let reference = match self.storage.load_last_media_reference() {
            Ok(reference) => reference,
            Err(error) => {
                log::warn!("try_resume_foreground: failed to read last media reference: {error}");
                self.session.report_external_error(
                    nerust_gui_shell::session::notification::FailureOrigin::Restore,
                    "Previous ROM reference is invalid; open it again".to_string(),
                );
                self.discard_restore_state();
                return;
            }
        };
        // Tiny local marker reads only; ROM bytes and hashes stay on the worker.
        let library_id = if reference.is_none() {
            self.storage.load_last_rom_id().unwrap_or_else(|error| {
                log::warn!("try_resume_foreground: failed to read last ROM id: {error}");
                None
            })
        } else {
            None
        };
        match crate::lifecycle_decision::restore_fetch_route(
            reference.is_some(),
            library_id.is_some(),
        ) {
            crate::lifecycle_decision::RestoreFetch::Document => {
                self.restore_from_document_uri(event_loop, reference.expect("routed on Some"))
            }
            crate::lifecycle_decision::RestoreFetch::Library => {
                self.restore_from_library_id(event_loop, library_id.expect("routed on Some"))
            }
            crate::lifecycle_decision::RestoreFetch::None => {
                log::info!("try_resume_foreground: no last ROM recorded");
                self.discard_restore_state();
            }
        }
    }

    /// Reconnect a document-tree URI; a missing document keeps the hidden snapshot until the user
    /// reconnects the tree. I/O runs on the fetch worker; the event thread only dispatches.
    fn restore_from_document_uri(
        &mut self,
        _event_loop: &ActiveEventLoop,
        reference: super::storage::LastMediaReference,
    ) {
        log::info!(
            "try_resume_foreground: fetching last document URI on the worker for lifecycle restore"
        );
        picker::spawn_document_fetch(self.app.clone(), reference.uri, true);
    }

    /// Reload the last library ROM by id, with hidden-state restore.
    /// I/O runs on the fetch worker; the event thread only dispatches.
    fn restore_from_library_id(&mut self, _event_loop: &ActiveEventLoop, id: String) {
        let Some(path) = self.storage.rom_library.rom_path(&id) else {
            log::warn!("try_resume_foreground: stored ROM id={id} is missing");
            self.discard_restore_state();
            return;
        };
        log::info!(
            "try_resume_foreground: fetching last ROM id={id} on the worker for lifecycle restore"
        );
        picker::spawn_library_fetch(path, id);
    }

    /// Drop pending restore flags plus the hidden lifecycle snapshot.
    pub(super) fn discard_restore_state(&mut self) {
        self.session.clear_hidden_lifecycle_state();
        self.storage.clear_restore_pending();
        self.lifecycle.clear_restore_pending();
    }
}
