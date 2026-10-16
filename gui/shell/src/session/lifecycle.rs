use std::{path::Path, sync::Arc};

use nerust_core_traits::factory::{
    CoreFactory,
    load::{MediaObject, ResolvedLoadRequest},
};
use nerust_emu_thread::ConsoleMetrics;
use nerust_input_traits::InputAssignments;
use nerust_settings_core::factory::settings_view;

use crate::{
    emu_core::EmuCore,
    session::{
        SessionError, SessionHandle,
        commands::{SessionCommand, SessionCommandOutcome, SlotOpFailure},
        persistence::PersistenceManager,
        title::window_title,
    },
};

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct WindowSize {
    pub width: f32,
    pub height: f32,
}

/// What `apply_live_runtime` did, so a failed settings save can undo
/// exactly that (rebuild vs swap-back) instead of guessing.
#[derive(Debug, Default)]
struct RuntimeApplyOutcome {
    rebuilt: bool,
    audio_swapped: bool,
    visual_swapped: bool,
}

impl SessionHandle {
    /// Volume/mute-only delta through the live gain control. Warn-only:
    /// a quiet failure must never block saving the setting.
    fn apply_live_volume(&self, next_settings: &nerust_gui_runtime::settings::SettingsSnapshot) {
        let volume = f32::from(next_settings.local.audio.master_volume_percent.min(100)) / 100.0;
        let volume = if next_settings.local.audio.muted {
            0.0
        } else {
            volume
        };
        if let Some(ref core) = self.emu_core
            && let Err(error) = core.set_volume(volume)
        {
            log::warn!("set_volume failed: {error}");
        }
    }

    /// Sample-rate/latency delta through a live backend swap. The
    /// replacement carries the new settings' volume, so volume-only
    /// deltas ride along for free.
    fn swap_audio_for_settings(
        &self,
        next_settings: &nerust_gui_runtime::settings::SettingsSnapshot,
    ) -> Result<(), SessionError> {
        let core = self.emu_core.as_ref().ok_or(SessionError::NoCore)?;
        let speaker = crate::settings::build_speaker(&self.audio_registry, &next_settings.local);
        let rate = speaker.sample_rate();
        core.swap_audio_backend(speaker, rate)?;
        Ok(())
    }

    /// Presentation-only visual delta through a live render-target
    /// swap. Returns whether the swap covered the delta; `false` keeps
    /// the full rebuild path (core behavior changed, or the factory
    /// names no presentation-only delta).
    fn swap_presentation_for_settings(
        &mut self,
        factory: &Arc<dyn CoreFactory>,
        previous: &nerust_gui_runtime::settings::SettingsSnapshot,
        next_settings: &nerust_gui_runtime::settings::SettingsSnapshot,
    ) -> Result<bool, SessionError> {
        let system_id = factory.system_id();
        let before_view = settings_view(previous, system_id.as_ref());
        let after_view = settings_view(next_settings, system_id.as_ref());
        let Some((profile, palette)) = factory.presentation_swap_assets(&before_view, &after_view)
        else {
            return Ok(false);
        };
        let core = self.emu_core.as_mut().ok_or(SessionError::NoCore)?;
        core.apply_presentation(profile, palette)?;
        Ok(true)
    }

    /// Runtime half of `apply_settings`: live swaps first
    /// (reply-barriered, no teardown, no ROM reload, no state
    /// round-trip), full reconstruction only for core behavior changes
    /// and reassignments.
    fn apply_live_runtime(
        &mut self,
        plan: &nerust_gui_runtime::settings::SettingsApplyPlan,
        factory: &Option<Arc<dyn CoreFactory>>,
        previous: &nerust_gui_runtime::settings::SettingsSnapshot,
        next_settings: &nerust_gui_runtime::settings::SettingsSnapshot,
        applied_assignments: &InputAssignments,
        assignments_changed: bool,
    ) -> Result<RuntimeApplyOutcome, SessionError> {
        // `session_rebuild_required` folds the rate/latency and visual
        // reasons; the live paths below absorb both, leaving only core
        // behavior changes (and reassignments) on the rebuild path.
        let visual_changed = plan.session_rebuild_required && !plan.audio_backend_changed;
        let mut outcome = RuntimeApplyOutcome::default();
        let mut needs_rebuild = visual_changed || assignments_changed;
        if plan.audio_backend_changed
            && !visual_changed
            && !assignments_changed
            && self.emu_core.is_some()
        {
            // Audio-only delta: the swap absorbs the rebuild entirely
            // (the replacement backend carries the new volume too).
            self.swap_audio_for_settings(next_settings)?;
            outcome.audio_swapped = true;
            needs_rebuild = false;
        }
        if needs_rebuild {
            let mut swapped = false;
            if visual_changed
                && !assignments_changed
                && self.emu_core.is_some()
                && let Some(factory) = factory
            {
                swapped = self.swap_presentation_for_settings(factory, previous, next_settings)?;
            }
            if swapped {
                outcome.visual_swapped = true;
                if plan.audio_backend_changed {
                    self.swap_audio_for_settings(next_settings)?;
                    outcome.audio_swapped = true;
                }
                if plan.audio_volume_changed {
                    self.apply_live_volume(next_settings);
                }
            } else {
                // Core behavior changed (or controllers reassigned):
                // full reconstruction carries the new speaker too.
                self.rebuild_for_settings(next_settings, applied_assignments)?;
                outcome.rebuilt = true;
            }
        } else if plan.audio_volume_changed && !outcome.audio_swapped {
            self.apply_live_volume(next_settings);
        }
        Ok(outcome)
    }

    /// Undo `apply_live_runtime` after a failed settings save.
    /// Rebuilds roll back by reconstructing the previous core; live
    /// swaps roll back through the same barriered path (cheap, no
    /// reconstruction). Every leg is warn-only: the save already
    /// failed, and a loud rollback must not mask it.
    fn rollback_runtime_apply(
        &mut self,
        factory: &Option<Arc<dyn CoreFactory>>,
        previous: &nerust_gui_runtime::settings::SettingsSnapshot,
        next_settings: &nerust_gui_runtime::settings::SettingsSnapshot,
        previous_assignments: &InputAssignments,
        outcome: &RuntimeApplyOutcome,
    ) {
        if outcome.rebuilt
            && let Err(e) = self.rebuild_for_settings(previous, previous_assignments)
        {
            log::warn!("failed to rollback settings rebuild: {e}");
            return;
        }
        if outcome.visual_swapped
            && let Some(factory) = factory
        {
            let system_id = factory.system_id();
            let before_view = settings_view(next_settings, system_id.as_ref());
            let after_view = settings_view(previous, system_id.as_ref());
            match factory.presentation_swap_assets(&before_view, &after_view) {
                Some((profile, palette)) => {
                    if let Some(core) = self.emu_core.as_mut()
                        && let Err(e) = core.apply_presentation(profile, palette)
                    {
                        log::warn!("failed to rollback presentation swap: {e}");
                    }
                }
                None => {
                    log::warn!("presentation swap rollback uncovered; keeping new assets");
                }
            }
        }
        if outcome.audio_swapped {
            let speaker = crate::settings::build_speaker(&self.audio_registry, &previous.local);
            let rate = speaker.sample_rate();
            if let Some(core) = self.emu_core.as_ref()
                && let Err(e) = core.swap_audio_backend(speaker, rate)
            {
                log::warn!("failed to rollback audio swap: {e}");
            }
        }
    }

    pub fn metrics(&self) -> ConsoleMetrics {
        self.emu_core
            .as_ref()
            .map(|c| c.metrics())
            .unwrap_or_default()
    }

    pub fn window_size(&self) -> WindowSize {
        let Some(core) = self.emu_core.as_ref() else {
            return WindowSize::default();
        };
        let profile = core.render_profile();
        WindowSize {
            width: profile.physical_size.width,
            height: profile.physical_size.height,
        }
    }

    pub fn window_title(&self) -> String {
        let metrics = self.metrics();
        let name = self.active_factory().map(|f| f.display_name());
        window_title(
            metrics.paused,
            metrics,
            name,
            self.active_slot_id(),
            self.slots().len(),
        )
    }

    pub fn loaded(&self) -> bool {
        self.metrics().loaded
    }

    pub fn paused(&self) -> bool {
        self.metrics().paused
    }

    /// Re-acquire the audio backend stream (mobile OS lifecycle
    /// transitions can kill audio streams; the re-acquire is idempotent).
    /// No-op without a loaded core.
    pub fn restart_audio(&mut self) {
        if let Some(ref core) = self.emu_core
            && let Err(error) = core.restart_audio()
        {
            log::warn!("restart_audio failed: {error}");
        }
    }

    pub fn can_pause(&self) -> bool {
        let metrics = self.metrics();
        metrics.loaded && !metrics.paused
    }

    pub fn can_resume(&self) -> bool {
        let metrics = self.metrics();
        metrics.loaded && metrics.paused
    }

    pub fn apply_settings(
        &mut self,
        next_settings: nerust_gui_runtime::settings::SettingsSnapshot,
    ) -> Result<nerust_gui_runtime::settings::SettingsApplyPlan, SessionError> {
        let previous = self.settings_snapshot.clone();
        let previous_assignments = self.current_assignments.clone();
        let factory = self.active_factory().cloned();

        let next_assignments = factory.as_ref().map(|f| {
            let sid = f.system_id();
            Self::load_assignments(f, &next_settings, sid.as_ref())
        });

        let plan = nerust_gui_runtime::settings::apply::derive_apply_plan(
            &self.capabilities,
            &previous,
            &next_settings,
            self.active_system_id(),
        );

        let assignments_changed = next_assignments
            .as_ref()
            .is_some_and(|a| a.to_string_pairs() != previous_assignments.to_string_pairs());

        if plan.persistence_changed
            && let Some(ref core) = self.emu_core
        {
            self.persistence.flush_mapper_save(core)?;
        }

        let outcome = self.apply_live_runtime(
            &plan,
            &factory,
            &previous,
            &next_settings,
            next_assignments.as_ref().unwrap_or(&previous_assignments),
            assignments_changed,
        )?;

        if let Err(error) = self.settings.save_snapshot(next_settings.clone()) {
            self.rollback_runtime_apply(
                &factory,
                &previous,
                &next_settings,
                &previous_assignments,
                &outcome,
            );
            return Err(SessionError::Settings(error));
        }

        if assignments_changed && let Some(a) = next_assignments {
            self.current_assignments = a;
        }

        self.settings_snapshot = next_settings;
        if plan.persistence_changed && self.loaded() {
            let rom_path = self
                .loaded_media
                .as_ref()
                .and_then(|media| media.media.native_path().map(Path::to_path_buf));
            self.setup_persistence(rom_path.as_deref(), false);
        }
        self.pressed_keys.clear();
        self.clear_input();
        self.rebuild_key_field_map();
        Ok(plan)
    }

    pub fn set_fullscreen_default(
        &mut self,
        fullscreen: bool,
    ) -> Result<nerust_gui_runtime::settings::SettingsApplyPlan, SessionError> {
        if self.settings_snapshot.local.video.window.fullscreen_default == fullscreen {
            return Ok(nerust_gui_runtime::settings::SettingsApplyPlan::default());
        }

        let mut next_settings = self.settings_snapshot.clone();
        next_settings.local.video.window.fullscreen_default = fullscreen;
        let plan = nerust_gui_runtime::settings::apply::derive_apply_plan(
            &self.capabilities,
            &self.settings_snapshot,
            &next_settings,
            self.active_system_id(),
        );

        if let Err(error) = self.settings.save_snapshot(next_settings.clone()) {
            return Err(SessionError::Settings(error));
        }

        self.settings_snapshot = next_settings;
        Ok(plan)
    }

    pub fn load_resolved(
        &mut self,
        media: MediaObject,
        resolved: ResolvedLoadRequest,
    ) -> Result<(), SessionError> {
        {
            let Some(core) = self.emu_core.as_ref() else {
                return Err(SessionError::NoCore);
            };
            self.persistence.flush_mapper_save(core)?;
        }
        self.emu_core
            .as_mut()
            .ok_or(SessionError::NoCore)?
            .load(&media, Some(resolved.options))?;
        self.loaded_media = Some(super::LoadedMedia {
            media: media.clone(),
        });

        self.setup_persistence(media.native_path(), true);

        self.remember_last_successful_rom_directory(media.native_path());
        Ok(())
    }

    pub fn unload(&mut self) -> Result<(), SessionError> {
        if let Some(ref core) = self.emu_core {
            self.persistence.flush_mapper_save(core)?;
        }
        if let Some(ref mut core) = self.emu_core {
            core.unload()?;
        }
        self.loaded_media = None;
        self.persistence.reset();
        Ok(())
    }

    pub fn flush_before_exit(&mut self) {
        if let Some(ref core) = self.emu_core
            && let Err(error) = self.persistence.flush_mapper_save(core)
        {
            log::warn!("mapper save flush before close failed: {error}");
        }
        // Persist the latest settings to disk.  Reload from the manager first
        // so that any pending changes (e.g. window size from
        // remember_fit_window_size) are not overwritten by a stale snapshot.
        match self.settings.reload() {
            Ok(snapshot) => {
                self.settings_snapshot = snapshot;
            }
            Err(error) => {
                log::warn!("settings reload before exit failed: {error}");
            }
        }
        if let Err(error) = self.settings.save_snapshot(self.settings_snapshot.clone()) {
            log::warn!("settings save on exit failed: {error}");
        }
    }

    pub fn run_command(
        &mut self,
        command: SessionCommand,
    ) -> Result<SessionCommandOutcome, SessionError> {
        // Resolve TogglePause to its steady arm so notifications name
        // the state actually entered.
        let was_paused = self.paused();
        let effective = match (command, was_paused) {
            (SessionCommand::TogglePause, false) => SessionCommand::Pause,
            (SessionCommand::TogglePause, true) => SessionCommand::Resume,
            (other, _) => other,
        };
        let result = self.run_command_inner(effective);
        match &result {
            Ok(outcome) => {
                let active = self.active_slot_id();
                for kind in super::notification::outcome_notifications(command, outcome, active) {
                    self.notifications.push(kind);
                }
            }
            Err(_) => {
                self.notifications
                    .push(super::notification::NotificationKind::CommandFailed(
                        command_name(command),
                    ))
            }
        }
        result
    }

    fn run_command_inner(
        &mut self,
        command: SessionCommand,
    ) -> Result<SessionCommandOutcome, SessionError> {
        match command {
            SessionCommand::Pause => self.cmd_pause(),
            SessionCommand::Resume => self.cmd_resume(),
            SessionCommand::TogglePause => self.cmd_toggle_pause(),
            SessionCommand::Reset => self.cmd_reset(),
            SessionCommand::CreateSlot => Ok(self.slot_op_ok(|p, c| p.create_slot(c))),
            SessionCommand::SaveActiveSlotOrNew => {
                Ok(self.slot_op_ok(|p, c| p.save_active_slot_or_new(c)))
            }
            SessionCommand::LoadActiveSlot => Ok(self.load_slot_op(|p, c| p.load_active_slot(c))),
            SessionCommand::SelectActiveSlot(id) => Ok(self.cmd_select_active_slot(id)),
            SessionCommand::SaveSlot(id) => Ok(self.slot_op_ok(|p, c| p.save_slot(id, c, false))),
            SessionCommand::LoadSlot(id) => Ok(self.load_slot_op(|p, c| p.load_slot(id, c))),
            SessionCommand::DeleteSlot(id) => Ok(self.slot_op(|p, c| p.delete_slot(id, c))),
            SessionCommand::SelectNextSlot => Ok(self.cmd_adjacent_slot(true)),
            SessionCommand::SelectPreviousSlot => Ok(self.cmd_adjacent_slot(false)),
        }
    }

    pub fn drain_notifications(&mut self) -> Vec<super::notification::UiNotification> {
        self.notifications.drain()
    }

    pub fn notification_history(
        &self,
    ) -> impl Iterator<Item = &super::notification::UiNotification> {
        self.notifications.history()
    }

    pub fn notification_history_lines(
        &self,
        language: nerust_settings_core::model::language::AppLanguage,
    ) -> Vec<super::osd::HistoryLine> {
        self.notifications
            .history()
            .map(|n| {
                let mut text = n.kind.text(language);
                if n.count > 1 {
                    text = format!("{text} ×{}", n.count);
                }
                super::osd::HistoryLine {
                    time: super::osd::format_time(n.at),
                    level: n.level,
                    text,
                    count: n.count,
                }
            })
            .collect()
    }

    pub fn clear_notification_history(&mut self) {
        self.notifications.clear_history();
    }

    fn cmd_pause(&mut self) -> Result<SessionCommandOutcome, SessionError> {
        if self.paused() {
            return Ok(SessionCommandOutcome::default());
        }
        self.core_mut()?.pause()?;
        Ok(SessionCommandOutcome {
            executed: true,
            needs_redraw: false,
            slot_failure: None,
        })
    }
}

fn command_name(command: SessionCommand) -> &'static str {
    match command {
        SessionCommand::Pause => "Pause",
        SessionCommand::Resume => "Resume",
        SessionCommand::TogglePause => "TogglePause",
        SessionCommand::Reset => "Reset",
        SessionCommand::CreateSlot => "CreateSlot",
        SessionCommand::SaveActiveSlotOrNew => "SaveActiveSlotOrNew",
        SessionCommand::LoadActiveSlot => "LoadActiveSlot",
        SessionCommand::SelectActiveSlot(_) => "SelectActiveSlot",
        SessionCommand::SaveSlot(_) => "SaveSlot",
        SessionCommand::LoadSlot(_) => "LoadSlot",
        SessionCommand::DeleteSlot(_) => "DeleteSlot",
        SessionCommand::SelectNextSlot => "SelectNextSlot",
        SessionCommand::SelectPreviousSlot => "SelectPreviousSlot",
    }
}

impl SessionHandle {
    fn cmd_resume(&mut self) -> Result<SessionCommandOutcome, SessionError> {
        if !self.paused() {
            return Ok(SessionCommandOutcome::default());
        }
        self.core_mut()?.resume()?;
        Ok(SessionCommandOutcome {
            executed: true,
            needs_redraw: self.loaded(),
            slot_failure: None,
        })
    }

    fn cmd_toggle_pause(&mut self) -> Result<SessionCommandOutcome, SessionError> {
        if self.paused() {
            self.cmd_resume()
        } else {
            self.cmd_pause()
        }
    }

    fn cmd_reset(&mut self) -> Result<SessionCommandOutcome, SessionError> {
        self.core_mut()?.reset()?;
        Ok(SessionCommandOutcome {
            executed: true,
            needs_redraw: false,
            slot_failure: None,
        })
    }

    fn cmd_select_active_slot(&mut self, slot_id: u64) -> SessionCommandOutcome {
        self.persistence.select_active_slot(slot_id);
        SessionCommandOutcome {
            executed: true,
            needs_redraw: false,
            slot_failure: None,
        }
    }

    fn cmd_adjacent_slot(&mut self, forward: bool) -> SessionCommandOutcome {
        SessionCommandOutcome {
            executed: self.persistence.select_adjacent_slot(forward).is_some(),
            needs_redraw: false,
            slot_failure: None,
        }
    }

    fn core_mut(&mut self) -> Result<&EmuCore, SessionError> {
        self.emu_core.as_ref().ok_or(SessionError::NoCore)
    }

    fn slot_op(
        &mut self,
        op: impl FnOnce(&mut PersistenceManager, &EmuCore),
    ) -> SessionCommandOutcome {
        if let Some(ref core) = self.emu_core {
            op(&mut self.persistence, core);
        }
        SessionCommandOutcome {
            executed: true,
            needs_redraw: false,
            slot_failure: None,
        }
    }

    fn slot_op_ok(
        &mut self,
        op: impl FnOnce(&mut PersistenceManager, &EmuCore) -> Result<(), SlotOpFailure>,
    ) -> SessionCommandOutcome {
        let slot_failure = if let Some(ref core) = self.emu_core {
            op(&mut self.persistence, core).err()
        } else {
            Some(SlotOpFailure::Unavailable)
        };
        SessionCommandOutcome {
            executed: slot_failure.is_none(),
            needs_redraw: false,
            slot_failure,
        }
    }

    fn load_slot_op(
        &mut self,
        op: impl FnOnce(&mut PersistenceManager, &EmuCore) -> Result<(), SlotOpFailure>,
    ) -> SessionCommandOutcome {
        let was_paused = self.paused();
        let slot_failure = if let Some(ref core) = self.emu_core {
            op(&mut self.persistence, core).err()
        } else {
            Some(SlotOpFailure::Empty)
        };
        let executed = slot_failure.is_none();
        SessionCommandOutcome {
            executed,
            needs_redraw: executed && was_paused && !self.paused(),
            slot_failure,
        }
    }

    pub fn slots(&self) -> &[nerust_persistence::model::StateSlotSummary] {
        self.persistence.slots()
    }

    pub fn active_slot_id(&self) -> Option<u64> {
        self.persistence.active_slot_id()
    }

    pub fn save_hidden_lifecycle_state(&mut self) -> bool {
        self.emu_core
            .as_ref()
            .is_some_and(|core| self.persistence.save_hidden(core))
    }

    pub fn load_hidden_lifecycle_state(&mut self) -> bool {
        self.emu_core
            .as_ref()
            .is_some_and(|core| self.persistence.load_hidden(core))
    }

    pub fn clear_hidden_lifecycle_state(&mut self) {
        self.persistence.clear_hidden();
    }

    pub(super) fn rebuild_for_settings(
        &mut self,
        next_settings: &nerust_gui_runtime::settings::SettingsSnapshot,
        assignments: &InputAssignments,
    ) -> Result<(), SessionError> {
        let was_loaded = self.loaded();
        let was_paused = self.paused();
        let exported_core_bytes = if was_loaded {
            Some(
                self.emu_core
                    .as_ref()
                    .ok_or(SessionError::NoCore)?
                    .save_state_raw()?,
            )
        } else {
            None
        };
        let restored_runtime_state = exported_core_bytes.is_some();

        let factory = self.active_factory().ok_or(SessionError::NoCore)?.clone();
        let rebuilt = self.build_core_for_settings(next_settings, &factory, assignments)?;
        let rebuilt_core = rebuilt.emu_core;

        if let Some(loaded_media) = self.loaded_media.clone() {
            rebuilt_core.load(&loaded_media.media, None)?;
            if let Some(core_bytes) = exported_core_bytes.as_ref() {
                rebuilt_core.load_state_raw(core_bytes.clone())?;
                if !was_paused {
                    rebuilt_core.resume()?;
                }
            }
        }

        self.emu_core = Some(rebuilt_core);
        self.gui_input = Some(rebuilt.gui_input);
        self.field_map = rebuilt.field_map;
        if was_loaded {
            let rom_path = self
                .loaded_media
                .as_ref()
                .and_then(|m| m.media.native_path().map(Path::to_path_buf));
            let rom_path_deref = rom_path.as_deref();
            self.setup_persistence(rom_path_deref, !restored_runtime_state);
            if was_paused && let Some(ref mut core) = self.emu_core {
                core.pause()?;
            }
        }
        Ok(())
    }

    fn build_core_for_settings(
        &self,
        next_settings: &nerust_gui_runtime::settings::SettingsSnapshot,
        factory: &Arc<dyn CoreFactory>,
        assignments: &InputAssignments,
    ) -> Result<super::CoreRuntime, SessionError> {
        let speaker = crate::settings::build_speaker(&self.audio_registry, &next_settings.local);
        let system_id = factory.system_id();
        let view = settings_view(next_settings, system_id.as_ref());
        let parts =
            factory.create_core_and_adapter_with_assignments(&view, speaker, assignments)?;
        Ok(super::CoreRuntime::from_factory_parts(parts))
    }

    fn resolve_persistence_paths(
        &self,
        rom_path: Option<&Path>,
        identity: &nerust_core_traits::identity::SystemIdentity,
    ) -> Option<nerust_persistence::sidecar::SidecarPaths> {
        self.settings
            .resolve_persistence_paths_with_import(identity.system_id.as_ref(), rom_path, identity)
            .map_err(|error| {
                log::warn!("failed to resolve persistence paths: {error}");
                error
            })
            .ok()
    }

    fn setup_persistence(&mut self, rom_path: Option<&Path>, load_mapper_save: bool) -> bool {
        let sidecars = self
            .emu_core
            .as_ref()
            .and_then(|c| c.canonical_media_identity())
            .and_then(|identity| self.resolve_persistence_paths(rom_path, &identity));
        let Some(sidecars) = sidecars else {
            return false;
        };
        self.persistence
            .configure(sidecars.states_dir, sidecars.mapper_save_path);
        if let Some(ref core) = self.emu_core {
            self.persistence.refresh_slots(core);
            if load_mapper_save && let Err(e) = self.persistence.load_mapper_save_if_needed(core) {
                log::warn!("load_mapper_save_if_needed failed: {e}");
            }
        }
        true
    }

    fn remember_last_successful_rom_directory(&mut self, path: Option<&Path>) {
        if let Some(path) = path
            && let Err(error) = self.settings.update_last_successful_rom_directory(path)
        {
            log::warn!("failed to update app state: {error}");
        }
        if let Ok(snapshot) = self.settings.snapshot() {
            self.settings_snapshot = snapshot;
        }
    }
}
