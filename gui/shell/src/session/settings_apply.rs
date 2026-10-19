//! Settings-driven session reconstruction: live swaps, full rebuilds,
//! and rollback. `SessionHandle` delegates here so the plan/apply/undo
//! protocol lives in one domain module.

use std::{path::Path, sync::Arc};

use nerust_core_traits::factory::CoreFactory;
use nerust_input_traits::InputAssignments;

use crate::session::{SessionError, SessionHandle};
use nerust_settings_core::factory::settings_view;

/// What `apply_live_runtime` did, so a failed settings save can undo
/// exactly that (rebuild vs swap-back) instead of guessing.
#[derive(Debug, Default)]
struct RuntimeApplyOutcome {
    rebuilt: bool,
    audio_swapped: bool,
    visual_swapped: bool,
}

impl SessionHandle {
    fn apply_live_volume(&self, next_settings: &nerust_gui_runtime::settings::SettingsSnapshot) {
        let volume = f32::from(next_settings.local.audio.master_volume_percent.min(100)) / 100.0;
        let volume = if next_settings.local.audio.muted {
            0.0
        } else {
            volume
        };
        if let Some(core) = self.emu.core_opt()
            && let Err(error) = core.set_volume(volume)
        {
            log::warn!("set_volume failed: {error}");
        }
    }

    fn swap_audio_for_settings(
        &self,
        next_settings: &nerust_gui_runtime::settings::SettingsSnapshot,
    ) -> Result<(), SessionError> {
        let core = self.emu.core()?;
        let speaker = crate::settings::build_speaker(&self.audio_registry, &next_settings.local);
        let rate = speaker.sample_rate();
        core.swap_audio_backend(speaker, rate)?;
        Ok(())
    }

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
        let core = self.emu.core_mut()?;
        core.apply_presentation(profile, palette)?;
        Ok(true)
    }

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
            && self.emu.core_opt().is_some()
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
                && self.emu.core_opt().is_some()
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
                    if let Some(core) = self.emu.core_opt_mut()
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
            if let Some(core) = self.emu.core_opt()
                && let Err(e) = core.swap_audio_backend(speaker, rate)
            {
                log::warn!("failed to rollback audio swap: {e}");
            }
        }
    }

    pub fn apply_settings(
        &mut self,
        next_settings: nerust_gui_runtime::settings::SettingsSnapshot,
    ) -> Result<nerust_gui_runtime::settings::SettingsApplyPlan, SessionError> {
        let previous = self.settings.snapshot.clone();
        let previous_assignments = self.input.current_assignments.clone();
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
            && let Some(core) = self.emu.core_opt()
        {
            self.media.persistence.flush_mapper_save(core)?;
        }

        let outcome = self.apply_live_runtime(
            &plan,
            &factory,
            &previous,
            &next_settings,
            next_assignments.as_ref().unwrap_or(&previous_assignments),
            assignments_changed,
        )?;

        if let Err(error) = self.settings.manager.save_snapshot(next_settings.clone()) {
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
            self.input.current_assignments = a;
        }

        self.settings.snapshot = next_settings;
        if plan.persistence_changed && self.loaded() {
            let rom_path = self
                .media
                .loaded
                .as_ref()
                .and_then(|media| media.media.native_path().map(Path::to_path_buf));
            let setup = self.setup_persistence(rom_path.as_deref(), false);
            self.note_persistence_setup(setup);
        }
        self.input.pressed_keys.clear();
        self.clear_input();
        self.rebuild_key_field_map();
        Ok(plan)
    }

    pub fn set_fullscreen_default(
        &mut self,
        fullscreen: bool,
    ) -> Result<nerust_gui_runtime::settings::SettingsApplyPlan, SessionError> {
        if self.settings.snapshot.local.video.window.fullscreen_default == fullscreen {
            return Ok(nerust_gui_runtime::settings::SettingsApplyPlan::default());
        }

        let mut next_settings = self.settings.snapshot.clone();
        next_settings.local.video.window.fullscreen_default = fullscreen;
        let plan = nerust_gui_runtime::settings::apply::derive_apply_plan(
            &self.capabilities,
            &self.settings.snapshot,
            &next_settings,
            self.active_system_id(),
        );

        if let Err(error) = self.settings.manager.save_snapshot(next_settings.clone()) {
            return Err(SessionError::Settings(error));
        }

        self.settings.snapshot = next_settings;
        Ok(plan)
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
                self.emu
                    .core_opt()
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

        if let Some(loaded_media) = self.media.loaded.clone() {
            rebuilt_core.load(&loaded_media.media, None)?;
            if let Some(core_bytes) = exported_core_bytes.as_ref() {
                rebuilt_core.load_state_raw(core_bytes.clone())?;
                if !was_paused {
                    rebuilt_core.resume()?;
                }
            }
        }

        self.emu.install(rebuilt_core, rebuilt.gui_input);
        self.input.field_map = rebuilt.field_map;
        if was_loaded {
            let rom_path = self
                .media
                .loaded
                .as_ref()
                .and_then(|m| m.media.native_path().map(Path::to_path_buf));
            let rom_path_deref = rom_path.as_deref();
            let setup = self.setup_persistence(rom_path_deref, !restored_runtime_state);
            self.note_persistence_setup(setup);
            if was_paused && let Some(core) = self.emu.core_opt_mut() {
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

    pub(crate) fn remember_last_successful_rom_directory(&mut self, path: Option<&Path>) {
        if let Some(path) = path
            && let Err(error) = self
                .settings
                .manager
                .update_last_successful_rom_directory(path)
        {
            log::warn!("failed to update app state: {error}");
        }
        if let Ok(snapshot) = self.settings.manager.snapshot() {
            self.settings.snapshot = snapshot;
        }
    }
}
