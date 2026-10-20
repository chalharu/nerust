//! Settings-driven session reconstruction: live swaps, full rebuilds, and rollback.
//! `SessionHandle` delegates here; the plan/apply/undo protocol lives in one domain module.

use std::{path::Path, sync::Arc};

use nerust_core_traits::factory::CoreFactory;
use nerust_gui_runtime::settings::{LiveStrategy, SettingsApplyPlan, SettingsSnapshot};
use nerust_input_traits::InputAssignments;

use crate::session::{
    SessionError, SessionHandle,
    osd::TickMetrics,
    persistence_setup::{MapperRestore, PersistenceBinder},
};
use nerust_settings_core::factory::settings_view;

/// Summary of a settings-apply operation: only flags frontends act on.
/// Home for the `SessionControl` result; `access` re-exports it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SettingsResult {
    /// The emulation core's renderer (GPU surface) needs to be recreated.
    pub renderer_needs_rebuild: bool,
    /// The fullscreen-default setting changed; frontend should sync the
    /// window's fullscreen state.
    pub fullscreen_default_changed: bool,
    /// The window-scaling mode changed; frontend should resize the window.
    pub scaling_changed: bool,
}

/// What [`ApplyRevert::revert`] undid; failures are reported here, never swallowed.
/// A failed rollback leaves the new runtime live for the caller to surface.
#[derive(Debug, Default)]
pub struct RollbackReport {
    pub restored_rebuild: bool,
    pub restored_presentation: bool,
    pub restored_audio: bool,
    /// First rollback failure, if any.
    pub error: Option<SessionError>,
}

impl RollbackReport {
    fn fail(&mut self, error: SessionError) {
        if self.error.is_none() {
            self.error = Some(error);
        }
    }
}

/// One snapshot of load-bearing runtime state, captured once per rebuild.
/// Single source for the export/restore decision below.
struct LoadedState {
    present: bool,
    core_bytes: Option<Vec<u8>>,
}

/// Derived apply inputs: the pure plan plus what the session resolved around it.
/// Splits derivation (testable, no `&mut`) from the mutating apply below.
struct DerivedApply {
    plan: SettingsApplyPlan,
    strategy: LiveStrategy,
    next_assignments: Option<InputAssignments>,
    assignments_changed: bool,
}

/// Bundled inputs for one live-runtime apply. Keeps the 8-field
/// call legible without a positional bool/enum trap.
struct LiveRuntimeInput<'a> {
    strategy: LiveStrategy,
    plan: &'a SettingsApplyPlan,
    factory: &'a Option<Arc<dyn CoreFactory>>,
    previous: &'a SettingsSnapshot,
    next_settings: &'a SettingsSnapshot,
    applied_assignments: &'a InputAssignments,
    previous_assignments: &'a InputAssignments,
    tick: &'a TickMetrics,
}

impl SessionHandle {
    fn apply_live_volume(&self, next_settings: &SettingsSnapshot) {
        let volume = f32::from(next_settings.local.audio.master_volume_percent.min(100)) / 100.0;
        let volume = if next_settings.local.audio.muted {
            0.0
        } else {
            volume
        };
        if let Some(core) = self.emu.core().ok()
            && let Err(error) = core.set_volume(volume)
        {
            log::warn!("set_volume failed: {error}");
        }
    }

    fn swap_audio_for_settings(
        &self,
        next_settings: &SettingsSnapshot,
    ) -> Result<(), SessionError> {
        let core = self.emu.core()?;
        let speaker = self.speaker_factory().build_speaker(&next_settings.local);
        let rate = speaker.sample_rate();
        core.swap_audio_backend(speaker, rate)?;
        Ok(())
    }

    fn swap_presentation_for_settings(
        &mut self,
        factory: &Arc<dyn CoreFactory>,
        previous: &SettingsSnapshot,
        next_settings: &SettingsSnapshot,
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

    /// Derive the plan, strategy, and assignments for `next_settings`.
    /// Pure read: no mutation, so tests can assert the strategy matrix without a live core.
    fn derive_apply(&self, next_settings: &SettingsSnapshot) -> DerivedApply {
        let previous_assignments = &self.input.current_assignments;
        let factory = self.active_factory_cloned();
        let next_assignments = factory.as_ref().map(|f| {
            let sid = f.system_id();
            Self::load_assignments(f, next_settings, sid.as_ref())
        });
        let plan = nerust_gui_runtime::settings::apply::derive_apply_plan(
            &self.capabilities,
            &self.settings.snapshot,
            next_settings,
            self.active_system_id(),
        );
        let assignments_changed = next_assignments
            .as_ref()
            .is_some_and(|a| a != previous_assignments);
        let strategy = plan.live_strategy(assignments_changed, self.emu.core().is_ok());
        DerivedApply {
            plan,
            strategy,
            next_assignments,
            assignments_changed,
        }
    }

    /// Run one derived strategy against the live runtime, returning the effective strategy.
    /// A refused presentation swap falls back to a rebuild, which is what gets undone.
    fn apply_live_runtime(
        &mut self,
        input: LiveRuntimeInput<'_>,
    ) -> Result<(LiveStrategy, ApplyRevert), SessionError> {
        let LiveRuntimeInput {
            strategy,
            plan,
            factory,
            previous,
            next_settings,
            applied_assignments,
            previous_assignments,
            tick,
        } = input;
        // Revert token skeleton: each arm fills the legs its strategy can undo.
        let mut revert = ApplyRevert {
            strategy,
            previous: None,
            previous_assignments: None,
            presentation_assets: None,
            audio: None,
        };
        let effective = match strategy {
            LiveStrategy::AudioSwap => {
                self.swap_audio_for_settings(next_settings)?;
                revert.audio = Some(previous_speaker(self, previous));
                LiveStrategy::AudioSwap
            }
            LiveStrategy::PresentationSwap => {
                let mut swapped = false;
                if let Some(factory) = factory {
                    swapped =
                        self.swap_presentation_for_settings(factory, previous, next_settings)?;
                    // Capture the swap-back assets with the forward pass, not at rollback time.
                    if swapped {
                        let system_id = factory.system_id();
                        let before_view = settings_view(next_settings, system_id.as_ref());
                        let after_view = settings_view(previous, system_id.as_ref());
                        revert.presentation_assets =
                            factory.presentation_swap_assets(&before_view, &after_view);
                    }
                }
                if swapped {
                    if plan.audio_backend_changed {
                        self.swap_audio_for_settings(next_settings)?;
                    }
                    LiveStrategy::PresentationSwap
                } else {
                    // Core behavior changed (or no factory to swap
                    // with): full reconstruction carries the new speaker too.
                    self.rebuild_for_settings(next_settings, applied_assignments, tick)?;
                    revert.previous = Some(previous.clone());
                    revert.previous_assignments = Some(previous_assignments.clone());
                    LiveStrategy::Rebuild
                }
            }
            LiveStrategy::Rebuild => {
                self.rebuild_for_settings(next_settings, applied_assignments, tick)?;
                revert.previous = Some(previous.clone());
                revert.previous_assignments = Some(previous_assignments.clone());
                LiveStrategy::Rebuild
            }
            LiveStrategy::Idle => LiveStrategy::Idle,
        };
        revert.strategy = effective;
        // The audio swap and the rebuild both carry the new volume;
        // only the presentation/idle paths need a separate nudge.
        if matches!(
            effective,
            LiveStrategy::Idle | LiveStrategy::PresentationSwap
        ) && plan.audio_volume_changed
        {
            self.apply_live_volume(next_settings);
        }
        Ok((effective, revert))
    }
}

/// Revert token for one applied strategy: everything `revert` needs, captured up front.
/// The forward pass pushes exactly one token; a save failure pops and runs it.
struct ApplyRevert {
    strategy: LiveStrategy,
    /// Previous snapshot for rebuild undo; cloned only on the rebuild path.
    previous: Option<SettingsSnapshot>,
    /// Previous assignments for rebuild undo; cloned only on the rebuild path.
    previous_assignments: Option<InputAssignments>,
    /// Previous presentation assets for swap-back; computed with the forward pass.
    presentation_assets: Option<(nerust_render_traits::VideoRenderProfile, Box<[u32]>)>,
    /// Previous speaker plus rate for audio swap-back; built with the forward pass.
    audio: Option<(Box<dyn nerust_core_traits::audio::AudioBackend>, u32)>,
}

impl ApplyRevert {
    /// Run the undo against the live session, reporting what each leg restored.
    /// A failed rollback leaves the new runtime live for the caller to surface.
    fn revert(self, session: &mut SessionHandle) -> RollbackReport {
        let mut report = RollbackReport::default();
        if self.strategy == LiveStrategy::Rebuild {
            let (Some(previous), Some(previous_assignments)) =
                (self.previous.as_ref(), self.previous_assignments.as_ref())
            else {
                log::error!("rebuild revert token missing previous runtime");
                return report;
            };
            // Tick is lock-free now: the pass tick re-reads without locking.
            let tick = session.tick_metrics();
            if let Err(e) = session.rebuild_for_settings(previous, previous_assignments, &tick) {
                log::error!("failed to rollback settings rebuild: {e}");
                report.fail(e);
                return report;
            }
        }
        report.restored_rebuild = self.strategy == LiveStrategy::Rebuild;
        if self.strategy == LiveStrategy::PresentationSwap {
            match self.presentation_assets {
                Some((profile, palette)) => {
                    if let Some(core) = session.emu.core_mut().ok()
                        && let Err(e) = core.apply_presentation(profile, palette)
                    {
                        log::error!("failed to rollback presentation swap: {e}");
                        report.fail(e.into());
                    } else {
                        report.restored_presentation = true;
                    }
                }
                None => {
                    log::warn!("presentation swap rollback uncovered; keeping new assets");
                }
            }
        }
        if self.strategy == LiveStrategy::AudioSwap
            && let Some((speaker, rate)) = self.audio
        {
            if let Some(core) = session.emu.core().ok()
                && let Err(e) = core.swap_audio_backend(speaker, rate)
            {
                log::error!("failed to rollback audio swap: {e}");
                report.fail(e.into());
            } else {
                report.restored_audio = true;
            }
        }
        report
    }
}

/// Undo stack for one settings apply: forward passes push revert tokens, failures pop them.
#[derive(Default)]
struct ApplyUndoStack(Vec<ApplyRevert>);

impl ApplyUndoStack {
    /// Push one forward pass's revert token.
    fn push_revert(&mut self, revert: ApplyRevert) {
        self.0.push(revert);
    }

    /// Pop the latest token for rollback. Empty outside a failed apply.
    fn pop_revert(&mut self) -> Option<ApplyRevert> {
        self.0.pop()
    }
}

impl SessionHandle {
    /// Persist `next_settings` and absorb it into the live session.
    /// Derivation, runtime apply, and input rebind run as named stages.
    pub fn apply_settings(
        &mut self,
        next_settings: SettingsSnapshot,
    ) -> Result<SettingsApplyPlan, SessionError> {
        let plan = self.apply_derived(next_settings)?;
        Ok(plan)
    }

    fn apply_derived(
        &mut self,
        next_settings: SettingsSnapshot,
    ) -> Result<SettingsApplyPlan, SessionError> {
        let previous = self.settings.snapshot.clone();
        let previous_assignments = self.input.current_assignments.clone();
        let factory = self.active_factory_cloned();
        let derived = self.derive_apply(&next_settings);
        // One lock-free tick for the whole apply: the tick threads
        // through the runtime apply and the persistence re-bind below.
        let tick = self.tick_metrics();

        if derived.plan.persistence_changed
            && let Ok(exporter) = self.emu.exporter()
        {
            self.media.persistence.flush_mapper_save(exporter)?;
        }

        let (_effective, revert) = self.apply_live_runtime(LiveRuntimeInput {
            strategy: derived.strategy,
            plan: &derived.plan,
            factory: &factory,
            previous: &previous,
            next_settings: &next_settings,
            applied_assignments: derived
                .next_assignments
                .as_ref()
                .unwrap_or(&previous_assignments),
            previous_assignments: &previous_assignments,
            tick: &tick,
        })?;
        // Stack discipline: the forward pass pushed one token; only a save failure pops it.
        let mut undo = ApplyUndoStack::default();
        undo.push_revert(revert);

        if let Err(error) = self.settings.manager.save_snapshot(next_settings.clone()) {
            if let Some(revert) = undo.pop_revert() {
                let report = revert.revert(self);
                if let Some(rollback_error) = report.error {
                    log::error!("settings rollback failed: {rollback_error}");
                }
            }
            return Err(SessionError::Settings(error));
        }

        if derived.assignments_changed
            && let Some(a) = derived.next_assignments
        {
            self.input.current_assignments = a;
        }

        self.settings.snapshot = next_settings;
        if derived.plan.persistence_changed && tick.loaded {
            let rom_path = self
                .media
                .loaded
                .as_ref()
                .and_then(|media| media.media.native_path().map(Path::to_path_buf));
            PersistenceBinder::bind(self, rom_path.as_deref(), MapperRestore::Skip);
        }
        self.rebind_input();
        Ok(derived.plan)
    }

    /// Rebuild the keyboard reverse map after anything that moves
    /// bindings, assignments, or controllers.
    fn rebind_input(&mut self) {
        self.input.pressed_keys.clear();
        self.clear_input();
        self.rebuild_key_field_map();
    }

    pub fn set_fullscreen_default(
        &mut self,
        fullscreen: bool,
    ) -> Result<SettingsApplyPlan, SessionError> {
        if self.settings.snapshot.local.video.window.fullscreen_default == fullscreen {
            return Ok(SettingsApplyPlan::default());
        }

        let mut next_settings = self.settings.snapshot.clone();
        next_settings.local.video.window.fullscreen_default = fullscreen;
        self.apply_settings(next_settings)
    }

    pub(super) fn rebuild_for_settings(
        &mut self,
        next_settings: &SettingsSnapshot,
        assignments: &InputAssignments,
        tick: &TickMetrics,
    ) -> Result<(), SessionError> {
        let loaded = LoadedState {
            present: tick.loaded || self.media.loaded.is_some(),
            core_bytes: tick
                .loaded
                .then(|| {
                    use crate::emu_core::LivePersistPort;
                    let core = self.emu.core().ok().ok_or(SessionError::NoCore)?;
                    LivePersistPort::save_state_raw(core)
                        .map_err(|error| error.slot_failure())
                        .map_err(SessionError::from)
                })
                .transpose()?,
        };

        let factory = self.active_factory_cloned().ok_or(SessionError::NoCore)?;
        let rebuilt =
            build_core_for_settings(self.speaker_factory(), next_settings, &factory, assignments)?;
        let rebuilt_core = rebuilt.emu_core;
        // The export decides the restore: a captured snapshot round-trips the runtime.
        // No capture means the disk mapper save is the only continuity.
        let restored_runtime_state = loaded.core_bytes.is_some();

        if loaded.present
            && let Some(media) = self.media.loaded.as_ref().map(|loaded| &loaded.media)
        {
            // Borrow, don't clone: `MediaObject` bytes ride an `Arc`,
            // but even the `PathBuf` half need not duplicate per rebuild.
            rebuilt_core.load(media, None)?;
            if let Some(core_bytes) = loaded.core_bytes {
                // Move, don't clone: the snapshot is hundreds of KB.
                rebuilt_core.load_state_raw(core_bytes)?;
                if !tick.paused {
                    rebuilt_core.resume()?;
                }
            }
        }

        self.emu.install(Box::new(rebuilt_core), rebuilt.gui_input);
        self.input.field_map = rebuilt.field_map;
        if loaded.present {
            let rom_path = self
                .media
                .loaded
                .as_ref()
                .and_then(|m| m.media.native_path().map(Path::to_path_buf));
            let restore = if restored_runtime_state {
                MapperRestore::Skip
            } else {
                MapperRestore::Load
            };
            PersistenceBinder::bind(self, rom_path.as_deref(), restore);
            if tick.paused
                && let Ok(core) = self.emu.core_mut()
            {
                core.pause()?;
            }
        }
        Ok(())
    }

    pub(crate) fn remember_last_successful_rom_directory(&mut self, path: Option<&Path>) {
        let Some(path) = path else { return };
        if let Err(error) = self
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

/// Previous speaker plus rate for audio swap-back, built with the forward pass.
fn previous_speaker(
    session: &SessionHandle,
    previous: &SettingsSnapshot,
) -> (Box<dyn nerust_core_traits::audio::AudioBackend>, u32) {
    let speaker = session.speaker_factory().build_speaker(&previous.local);
    let rate = speaker.sample_rate();
    (speaker, rate)
}

/// Build a fresh core for `next_settings` plus `assignments`, with no session borrowing.
/// Free function of exactly what the build consumes, so callers hold the session borrow.
fn build_core_for_settings(
    speaker_factory: &dyn crate::settings::AudioSpeakerFactory,
    next_settings: &SettingsSnapshot,
    factory: &Arc<dyn CoreFactory>,
    assignments: &InputAssignments,
) -> Result<super::CoreRuntime, SessionError> {
    let speaker = speaker_factory.build_speaker(&next_settings.local);
    let system_id = factory.system_id();
    let view = settings_view(next_settings, system_id.as_ref());
    let parts = factory.create_core_and_adapter_with_assignments(&view, speaker, assignments)?;
    Ok(super::CoreRuntime::from_factory_parts(parts))
}

#[cfg(test)]
mod settings_apply_cov_tests {
    use crate::session::settings_apply::*;
    use crate::session::test_util::test_session;

    #[test]
    fn rollback_report_keeps_first_error() {
        let mut report = RollbackReport::default();
        report.fail(SessionError::NoCore);
        report.fail(SessionError::NoCore);
        assert!(report.error.is_some());
        assert!(!report.restored_rebuild);
        assert!(!report.restored_presentation);
        assert!(!report.restored_audio);
    }

    #[test]
    fn undo_stack_push_pop_discipline() {
        let mut stack = ApplyUndoStack::default();
        assert!(stack.pop_revert().is_none());
        stack.push_revert(ApplyRevert {
            strategy: LiveStrategy::Idle,
            previous: None,
            previous_assignments: None,
            presentation_assets: None,
            audio: None,
        });
        assert!(stack.pop_revert().is_some());
        assert!(stack.pop_revert().is_none());
    }

    #[test]
    fn idle_revert_restores_nothing_without_error() {
        let mut session = test_session();
        let report = ApplyRevert {
            strategy: LiveStrategy::Idle,
            previous: None,
            previous_assignments: None,
            presentation_assets: None,
            audio: None,
        }
        .revert(&mut session);
        assert!(report.error.is_none());
        assert!(!report.restored_rebuild);
        assert!(!report.restored_presentation);
        assert!(!report.restored_audio);
    }

    #[test]
    fn rebuild_revert_without_token_reports_clean() {
        let mut session = test_session();
        let report = ApplyRevert {
            strategy: LiveStrategy::Rebuild,
            previous: None,
            previous_assignments: None,
            presentation_assets: None,
            audio: None,
        }
        .revert(&mut session);
        assert!(report.error.is_none());
        assert!(!report.restored_rebuild);
    }

    #[test]
    fn presentation_audio_reverts_without_assets_restore_nothing() {
        let mut session = test_session();
        for strategy in [LiveStrategy::PresentationSwap, LiveStrategy::AudioSwap] {
            let report = ApplyRevert {
                strategy,
                previous: None,
                previous_assignments: None,
                presentation_assets: None,
                audio: None,
            }
            .revert(&mut session);
            assert!(report.error.is_none());
        }
    }

    #[test]
    fn fullscreen_default_noop_when_unchanged() {
        let mut session = test_session();
        let current = session
            .settings_snapshot()
            .local
            .video
            .window
            .fullscreen_default;
        let plan = session.set_fullscreen_default(current).unwrap();
        assert_eq!(plan, SettingsApplyPlan::default());
    }

    #[test]
    fn remember_directory_ignores_none() {
        let mut session = test_session();
        session.remember_last_successful_rom_directory(None);
    }

    #[test]
    fn derive_apply_reports_idle_without_changes() {
        let session = test_session();
        let snapshot = session.settings_snapshot().clone();
        let derived = session.derive_apply(&snapshot);
        assert_eq!(derived.strategy, LiveStrategy::Idle);
        assert!(!derived.assignments_changed);
    }
}
