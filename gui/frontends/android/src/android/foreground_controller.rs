//! ForegroundController: window-ready retry owned by one module.
//! Drives `Lifecycle`, `RenderSync`, and `Storage`; the frontend executes.

use std::time::Instant;

use nerust_gui_shell::session::access::PauseControl as _;
use winit::event_loop::ActiveEventLoop;

use super::picker;
use crate::lifecycle_decision::RestoreAction;

impl super::AndroidFrontend {
    pub(super) fn try_resume_foreground(&mut self, event_loop: &ActiveEventLoop) {
        if !self.is_resumed || !self.flow.lifecycle.resume_pending() {
            return;
        }
        if let Some(retry_at) = self.flow.lifecycle.retry_deadline() {
            if Instant::now() < retry_at {
                return;
            }
            self.flow.lifecycle.clear_retry_deadline();
        }
        log::info!(
            "try_resume_foreground: attempt {}",
            self.flow.lifecycle.attempt_index()
        );
        match self.flow.render_sync.ensure_window(
            event_loop,
            &self.core.session,
            &self.core.gpu_factory,
            &mut self.chrome.osd,
            &mut self.core.shell,
        ) {
            Ok(()) => self.finish_foreground_attempt(event_loop),
            Err(error) => {
                let attempt = self.flow.lifecycle.attempt_index();
                log::warn!("try_resume_foreground: attempt {attempt} failed: {error}");
                self.flow.lifecycle.schedule_retry(self.is_resumed);
            }
        }
    }

    /// Window is ready: restore lifecycle media, resume own auto-pause,
    /// and re-acquire audio. Error auto-pauses are never resumed here.
    fn finish_foreground_attempt(&mut self, event_loop: &ActiveEventLoop) {
        let attempt = self.flow.lifecycle.attempt_index();
        self.flow.lifecycle.record_success();
        // Single policy: the fetch lane dispatches, the retry gate resumes.
        let loaded = self.core.session.tick_metrics().loaded;
        let should_restore = crate::lifecycle_decision::should_fetch_restore(
            self.flow.lifecycle.restore_pending(),
            loaded,
            picker::has_pending_result(),
        );
        let (has_document, has_library) = if should_restore {
            self.restore_candidates()
        } else {
            (false, false)
        };
        let policy = self.flow.lifecycle.policy(
            should_restore,
            self.flow.dialogs.auto_paused(),
            has_document,
            has_library,
        );
        match policy.restore_action() {
            RestoreAction::FreshNone => {}
            RestoreAction::FetchDocument => self.restore_from_document_uri(event_loop),
            RestoreAction::FetchLibrary => self.restore_from_library_id(event_loop),
            RestoreAction::Discard => {
                log::info!("try_resume_foreground: no last ROM recorded");
                self.discard_restore_state();
            }
        }
        if loaded {
            // Keep the on-disk crash/restart marker, but no in-process media reload is needed.
            self.flow.lifecycle.clear_restore_pending();
        }
        if policy.should_resume_retry() {
            self.resume();
            self.flow.lifecycle.set_auto_paused(false);
            log::info!("try_resume_foreground: resumed session after lifecycle pause");
        }
        self.core.session.restart_audio();
        log::info!("try_resume_foreground: attempt {attempt} succeeded");
        self.flow.render_sync.request_redraw(&mut self.core.shell);
    }

    /// Tiny local marker reads only; ROM bytes and hashes stay on the worker.
    fn restore_candidates(&mut self) -> (bool, bool) {
        let reference = match self.core.storage.load_last_media_reference() {
            Ok(reference) => reference,
            Err(error) => {
                log::warn!("try_resume_foreground: failed to read last media reference: {error}");
                self.core.session.report_external_error(
                    nerust_gui_shell::session::notification::FailureOrigin::Restore,
                    "Previous ROM reference is invalid; open it again".to_string(),
                );
                self.discard_restore_state();
                return (false, false);
            }
        };
        if reference.is_some() {
            return (true, false);
        }
        let library_id = self
            .core
            .storage
            .load_last_rom_id()
            .unwrap_or_else(|error| {
                log::warn!("try_resume_foreground: failed to read last ROM id: {error}");
                None
            });
        (false, library_id.is_some())
    }

    /// Reconnect a document-tree URI; a missing document keeps the hidden snapshot until the user
    /// reconnects the tree. I/O runs on the fetch worker; the event thread only dispatches.
    fn restore_from_document_uri(&mut self, _event_loop: &ActiveEventLoop) {
        let Ok(Some(reference)) = self.core.storage.load_last_media_reference() else {
            log::warn!("try_resume_foreground: last media reference vanished");
            self.discard_restore_state();
            return;
        };
        log::info!(
            "try_resume_foreground: fetching last document URI on the worker for lifecycle restore"
        );
        picker::spawn_document_fetch(self.app.clone(), reference.uri, true);
    }

    /// Reload the last library ROM by id, with hidden-state restore.
    /// I/O runs on the fetch worker; the event thread only dispatches.
    fn restore_from_library_id(&mut self, _event_loop: &ActiveEventLoop) {
        let Ok(Some(id)) = self.core.storage.load_last_rom_id() else {
            log::warn!("try_resume_foreground: last ROM id vanished");
            self.discard_restore_state();
            return;
        };
        let Some(path) = self.core.storage.resolve_library_path(&id) else {
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
        self.core.session.clear_hidden_lifecycle_state();
        self.core.storage.clear_restore_pending();
        self.flow.lifecycle.clear_restore_pending();
    }
}
