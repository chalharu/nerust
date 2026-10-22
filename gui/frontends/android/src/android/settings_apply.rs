//! SettingsApply: dialog result gate, plan, and execution owned by one module.
//! The frontend orchestrates; this module owns the settings-apply flow.

use nerust_gui_shell::session::{notification::FailureOrigin, osd::TickMetrics};
use nerust_gui_shell::session::access::{PauseControl as _, SessionControl as _};

use nerust_gui_runtime::settings::SettingsSnapshot;

use super::{
    error_dialog, picker,
    settings::{self, AndroidSettings, SettingsApplier, SettingsDialogResult, SettingsPlan},
};
use crate::error_dialog_controller::SettingsClosedTransition;

impl super::AndroidFrontend {
    /// Gate a settings result: stale synthesized dismisses die here, applied
    /// values racing a live dialog still apply. Returns false to stop.
    fn gate_settings_result(&mut self, is_applied: bool, tick: TickMetrics) -> bool {
        if self.flow.dialogs.is_settings_open() && !is_applied {
            // Stale synthesized dismiss racing a suspend-then-rerequest: ignore it.
            // The live flight's result arrives with its own request id.
            log::debug!("handle_settings_result: stale result while dialog open; ignoring");
            return false;
        }
        if self.flow.dialogs.is_settings_open() {
            // Applied racing a live re-request: the values are real user input, so apply them.
            // Error flush stays shielded below; the user re-saves if needed.
            log::warn!("handle_settings_result: applying settings while a newer dialog is open");
        } else {
            match self.chrome.osd.with_modal(|queue| {
                self.flow
                    .dialogs
                    .on_settings_closed(queue, self.is_resumed, tick)
            }) {
                SettingsClosedTransition::Show { auto_pause } => {
                    if auto_pause {
                        self.pause();
                    }
                    if let (Some(id), Some(detail)) = (
                        self.flow.dialogs.current_id(),
                        self.flow.dialogs.current_detail(),
                    ) {
                        error_dialog::request_show_error_dialog(&self.app, id, detail);
                    }
                }
                SettingsClosedTransition::Resume => self.resume(),
                SettingsClosedTransition::Idle => {}
            }
        }
        true
    }

    pub(super) fn handle_settings_result(&mut self, result: SettingsDialogResult, tick: TickMetrics) {
        // Settings closed: flush the error queue and resume an orphaned pause.
        // The gate reads the injected screen source; no cached flag exists to go stale.
        let is_applied = matches!(result, SettingsDialogResult::Applied(_));
        if !self.gate_settings_result(is_applied, tick) {
            return;
        }
        let SettingsDialogResult::Applied(values) = result else {
            log::info!("handle_settings_result: settings dialog dismissed");
            return;
        };
        log::info!("handle_settings_result: applying Android settings");
        // Thread the dialog cache instead of rebuilding from the
        // snapshot; refresh paths keep it current.
        let mut current = settings::cloned_cached_settings().unwrap_or_else(|| {
            AndroidSettings::from_snapshot(
                self.core.session.settings_snapshot(),
                self.core.session.factories(),
            )
        });
        current.prioritize_system(self.core.session.active_system_id());
        // Pure plan step; the frontend only executes the outcome.
        // Defs come from the bridge cache; the plan never re-locks.
        let defs = settings::cached_setting_defs(&current);
        let plan = SettingsApplier::plan(
            &values,
            &current,
            &defs,
            self.core.session.settings_snapshot().clone(),
            self.core.session.factories(),
        );
        match plan {
            SettingsPlan::Invalid { reason } => {
                log::error!("{reason}");
                self.core
                    .session
                    .report_external_error(FailureOrigin::Host, reason);
            }
            SettingsPlan::NeedDocumentTree { next } => {
                self.pending_storage_settings = Some(next);
                match picker::request_open_document_tree(&self.app) {
                    Ok(true) => {}
                    Ok(false) => {
                        self.pending_storage_settings = None;
                        log::warn!("Android directory picker is already open");
                    }
                    Err(error) => {
                        self.pending_storage_settings = None;
                        log::error!("{error}");
                        self.core
                            .session
                            .report_external_error(FailureOrigin::Host, error);
                    }
                }
            }
            SettingsPlan::Apply { next } => self.execute_settings_apply(next),
        }
    }

    /// Execute an `Apply` plan: apply, rebuild renderer/overlay, refresh
    /// caches. JNI and session failures route to the error queue.
    pub(super) fn execute_settings_apply(&mut self, next: SettingsSnapshot) {
        match self.apply_settings(next) {
            Ok(result) => {
                if result.renderer_needs_rebuild {
                    // If Android has already dropped the surface, keep the renderer absent here;
                    // `ensure_window` will rebuild it on the next resume with the updated settings.
                    if let Some(window) = self.flow.render_sync.cloned_window()
                        && let Err(error) = self.flow.render_sync.rebuild_renderer(
                            window,
                            &self.core.session,
                            &self.core.gpu_factory,
                            &mut self.chrome.osd,
                            &mut self.core.shell,
                        )
                    {
                        log::warn!("settings renderer rebuild failed: {error}");
                        self.flow.lifecycle.begin_foreground_resume();
                        self.flow.lifecycle.schedule_retry(self.is_resumed);
                    }
                }
                // Push overlay and FPS-label settings to the live chrome.
                // Otherwise saved and visible settings diverge until the next load or resize.
                self.chrome.input.rebuild_overlay(
                    &self.core.session,
                    &self.app,
                    self.flow.render_sync.window_size(),
                );
                self.flow.render_sync.request_redraw(&mut self.core.shell);
                // Settings changed – refresh cached settings for sync dialogs.
                self.refresh_dialog_caches();
                // Peripheral settings may have changed: force a re-push
                // on the next pass (dirty generation).
                self.chrome.peripherals.mark_dirty();
            }
            Err(error) => {
                let detail = format!("failed to apply Android settings: {error}");
                log::error!("{detail}");
                self.core
                    .session
                    .report_external_error(FailureOrigin::Host, detail);
            }
        }
    }
}
