//! SettingsApply: accepted-snapshot gate, storage-picker plan, and execution.
//! The frontend orchestrates; this module owns the settings-apply flow.

use nerust_gui_runtime::settings::SettingsSnapshot;
use nerust_gui_shell::session::access::{PauseControl as _, SessionControl as _};
use nerust_gui_shell::session::{notification::FailureOrigin, osd::TickMetrics};
use nerust_settings_core::model::shared::StoragePolicy;

use super::{
    bridge, error_dialog, picker,
    settings::{self, SettingsDialogResult},
};

impl super::AndroidFrontend {
    /// Gate a settings result: stale synthesized dismisses die here, applied
    /// snapshots racing a live dialog still apply. Returns false to stop.
    fn gate_settings_result(&mut self, is_applied: bool, tick: TickMetrics) -> bool {
        if self.flow.dialogs.is_settings_open() && !is_applied {
            // Stale synthesized dismiss racing a suspend-then-rerequest: ignore it.
            // The live flight's result arrives with its own request id.
            log::debug!("handle_settings_result: stale result while dialog open; ignoring");
            return false;
        }
        if self.flow.dialogs.is_settings_open() {
            // Applied racing a live re-request: the snapshot is real user input, so apply it.
            // Error flush stays shielded below; the user re-saves if needed.
            log::warn!("handle_settings_result: applying settings while a newer dialog is open");
        } else if !is_applied {
            // Dismissed without changes: flush under the current presentation.
            self.flush_closed_settings_dialogs(tick);
        }
        true
    }

    /// Settings-close dialog flush: take the next queued batch after the session apply.
    fn flush_closed_settings_dialogs(&mut self, tick: TickMetrics) {
        match self.chrome.osd.with_modal(|queue| {
            self.flow
                .dialogs
                .on_settings_closed(queue, self.is_resumed, tick)
        }) {
            crate::error_dialog_controller::SettingsClosedTransition::Show { auto_pause } => {
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
            crate::error_dialog_controller::SettingsClosedTransition::Resume => self.resume(),
            crate::error_dialog_controller::SettingsClosedTransition::Idle => {}
        }
    }

    pub(super) fn handle_settings_result(
        &mut self,
        result: SettingsDialogResult,
        tick: TickMetrics,
    ) {
        // Settings closed: flush the error queue and resume an orphaned pause.
        // The gate reads the injected screen source; no cached flag exists to go stale.
        let is_applied = matches!(result, SettingsDialogResult::Applied(_));
        if !self.gate_settings_result(is_applied, tick) {
            return;
        }
        let SettingsDialogResult::Applied(next) = result else {
            log::info!("handle_settings_result: settings dialog dismissed");
            return;
        };
        log::info!("handle_settings_result: validating Android settings");
        let factories = self.core.session.factories().to_vec();
        let rates: std::sync::Arc<[u32]> = self
            .core
            .session
            .audio_registry()
            .supported_rates()
            .iter()
            .copied()
            .collect();
        let system_locale = bridge::with_settings(|state| state.system_locale);
        match settings::finish_snapshot(&next, &factories, rates, system_locale) {
            Ok(validated) => {
                // The drawer and every Android string follow the saved
                // choice: push the validated language, never a rejected
                // draft's.
                settings::apply_app_language(
                    &self.app,
                    settings::app_language_tag(validated.shared.general.language),
                );
                self.plan_storage_apply(validated)
            }
            Err(issues) => {
                let reason = format!(
                    "Android settings rejected by validation: {}",
                    issues.join("; ")
                );
                log::error!("{reason}");
                self.core
                    .session
                    .report_external_error(FailureOrigin::Host, reason);
            }
        }
        // Accepted settings are in: flush the dialog queue under the new presentation.
        if !self.flow.dialogs.is_settings_open() {
            self.flush_closed_settings_dialogs(tick);
        }
    }

    /// Storage-policy gate: a custom directory without a tree URI needs the
    /// picker first; the picked URI resumes through `handle_picker_result`.
    fn plan_storage_apply(&mut self, next: SettingsSnapshot) {
        let previous = self
            .core
            .session
            .settings_snapshot()
            .shared
            .persistence
            .storage_policy;
        if next.shared.persistence.storage_policy != StoragePolicy::AppSharedData
            && (next.shared.persistence.storage_policy != previous
                || next.shared.persistence.storage_document_tree_uri.is_none())
        {
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
            return;
        }
        let mut next = next;
        if next.shared.persistence.storage_policy == StoragePolicy::AppSharedData {
            next.shared.persistence.storage_document_tree_uri = None;
        }
        self.execute_settings_apply(next);
    }

    /// Execute an apply: rebuild renderer/overlay, refresh caches. JNI and
    /// session failures route to the error queue.
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
                            &mut self.core.gpu_factory,
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
