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
            Ok(()) => {
                let attempt = self.lifecycle.attempt_index();
                self.lifecycle.record_success();
                if self.lifecycle.restore_pending() {
                    log::info!(
                        "try_resume_foreground: lifecycle_restore_pending=true; attempting to load last ROM and restore hidden lifecycle state"
                    );
                    match self.storage.load_last_media_reference() {
                        Ok(Some(reference)) => {
                            match self.load_document_uri(event_loop, reference, true) {
                                Ok(()) => {
                                    log::info!(
                                        "try_resume_foreground: loaded last document URI for lifecycle restore"
                                    );
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
                        Ok(None) => match self.storage.load_last_rom_id() {
                            Ok(Some(id)) => {
                                if self.storage.rom_library.rom_path(&id).is_none() {
                                    log::warn!(
                                        "try_resume_foreground: stored ROM id={id} is missing"
                                    );
                                    self.session.clear_hidden_lifecycle_state();
                                    self.storage.clear_restore_pending();
                                    self.lifecycle.clear_restore_pending();
                                } else {
                                    match self
                                        .load_from_library_with_autosave(event_loop, &id, true)
                                    {
                                        Ok(()) => {
                                            log::info!(
                                                "try_resume_foreground: loaded last ROM id={id} for lifecycle restore"
                                            );
                                            // Toasts mark success only; failures surface via the error dialog.
                                            super::show_toast(
                                                &self.app,
                                                "Legacy ROM restored; use Open ROM to reconnect its document",
                                            );
                                            // finish_rom_load will handle resume and clearing pending flags.
                                        }
                                        Err(error) => {
                                            log::warn!(
                                                "try_resume_foreground: failed to load last ROM id={id} for lifecycle restore: {error}"
                                            );
                                            self.session.clear_hidden_lifecycle_state();
                                            self.storage.clear_restore_pending();
                                            self.lifecycle.clear_restore_pending();
                                        }
                                    }
                                }
                            }
                            Ok(None) => {
                                log::info!("try_resume_foreground: no last ROM recorded");
                                self.session.clear_hidden_lifecycle_state();
                                self.storage.clear_restore_pending();
                                self.lifecycle.clear_restore_pending();
                            }
                            Err(error) => {
                                log::warn!(
                                    "try_resume_foreground: failed to read last ROM id: {error}"
                                );
                                self.session.clear_hidden_lifecycle_state();
                                self.storage.clear_restore_pending();
                                self.lifecycle.clear_restore_pending();
                            }
                        },
                        Err(error) => {
                            log::warn!(
                                "try_resume_foreground: failed to read last media reference: {error}"
                            );
                            self.session.report_external_error(
                                nerust_gui_shell::session::notification::FailureOrigin::Restore,
                                "Previous ROM reference is invalid; open it again".to_string(),
                            );
                            self.session.clear_hidden_lifecycle_state();
                            self.storage.clear_restore_pending();
                            self.lifecycle.clear_restore_pending();
                        }
                    }
                }
                // Resume only our own lifecycle auto-pause, never across
                // an error auto-pause (its dismiss path owns that resume).
                if self
                    .lifecycle
                    .resume_want_foreground_retry(self.dialogs.auto_paused())
                {
                    self.resume();
                    self.lifecycle.set_auto_paused(false);
                    log::info!("try_resume_foreground: resumed session after lifecycle pause");
                }
                // The OS may have killed the audio stream while
                // backgrounded regardless of pause state; re-acquire it
                // (idempotent, silent when already playing).
                self.session.restart_audio();
                log::info!("try_resume_foreground: attempt {attempt} succeeded");
                self.render_sync.request_redraw(&mut self.shell);
            }
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
}
