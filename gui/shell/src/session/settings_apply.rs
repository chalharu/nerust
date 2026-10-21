//! Settings-driven session reconstruction: live swaps, full rebuilds, and rollback.
//! `SessionHandle` delegates here; the plan/apply/undo protocol lives in one domain module.

use std::{path::Path, sync::Arc};

use nerust_core_traits::audio::AudioBackend;
use nerust_gui_runtime::settings::{
    LiveStrategy, SettingsApplyPlan, SettingsSnapshot,
    apply::{validate_local_settings, validate_shared_settings},
};
use nerust_input_traits::InputAssignments;
use nerust_render_traits::VideoRenderProfile;
use nerust_system_traits::factory::CoreFactory;

use crate::session::{
    ActiveRuntime, SessionError, SessionHandle,
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

/// Derived apply inputs: the pure plan plus what the session resolved around it.
/// Splits derivation (testable, no `&mut`) from the mutating apply below.
struct DerivedApply {
    plan: SettingsApplyPlan,
    strategy: LiveStrategy,
    next_assignments: Option<InputAssignments>,
    assignments_changed: bool,
}

impl SessionHandle {
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

    /// Record the inverse of each successful operation immediately, including compound swaps.
    fn apply_live_runtime(
        &mut self,
        derived: &DerivedApply,
        next_settings: &SettingsSnapshot,
        tick: &TickMetrics,
        applied: &mut Vec<AppliedOperation>,
    ) -> Result<(), SessionError> {
        let mut rebuild = derived.strategy == LiveStrategy::Rebuild;
        if derived.strategy == LiveStrategy::PresentationSwap {
            // A one-way presentation swap cannot be undone. Resolve both directions first.
            let assets = self.active_factory_cloned().and_then(|factory| {
                let sid = factory.system_id();
                let before = settings_view(&self.settings.snapshot, sid.as_ref());
                let after = settings_view(next_settings, sid.as_ref());
                Some((
                    factory.presentation_swap_assets(&before, &after)?,
                    factory.presentation_swap_assets(&after, &before)?,
                ))
            });
            if let Some(((profile, palette), (old_profile, old_palette))) = assets {
                if let Err(error) = self.emu.core_mut()?.apply_presentation(profile, palette) {
                    // Queue rejection is the only proof that nothing changed. A missing
                    // reply or a compound failure may have already replaced the frame slot.
                    if !matches!(error, nerust_emu_thread::OperationError::WorkerUnavailable) {
                        applied.push(AppliedOperation::Presentation(old_profile, old_palette));
                    }
                    return Err(error.into());
                }
                applied.push(AppliedOperation::Presentation(old_profile, old_palette));
            } else {
                rebuild = true;
            }
        }
        if rebuild {
            if tick.loaded && !tick.paused {
                self.emu.core()?.pause()?;
                applied.push(AppliedOperation::ResumePrevious);
            }
            let assignments = derived
                .next_assignments
                .as_ref()
                .unwrap_or(&self.input.current_assignments);
            let next = self.prepare_runtime_for_settings(next_settings, assignments, tick)?;
            applied.push(AppliedOperation::Runtime(self.emu.replace(next)));
            return Ok(());
        }
        if self.emu.core().is_ok() && derived.plan.audio_backend_changed {
            let previous = self
                .speaker_factory()
                .build_speaker(&self.settings.snapshot.local);
            let previous_rate = previous.sample_rate();
            let speaker = self.speaker_factory().build_speaker(&next_settings.local);
            let rate = speaker.sample_rate();
            if let Err(error) = self.emu.core()?.swap_audio_backend(speaker, rate) {
                if !matches!(error, nerust_emu_thread::OperationError::WorkerUnavailable) {
                    applied.push(AppliedOperation::Audio(previous, previous_rate));
                }
                return Err(error.into());
            }
            applied.push(AppliedOperation::Audio(previous, previous_rate));
        } else if self.emu.core().is_ok() && derived.plan.audio_volume_changed {
            self.emu.core()?.set_volume(volume(next_settings))?;
            applied.push(AppliedOperation::Volume(volume(&self.settings.snapshot)));
        }
        Ok(())
    }
}

/// Each variant is the inverse of one actual operation, with no unrelated optional tokens.
enum AppliedOperation {
    Runtime(Option<ActiveRuntime>),
    ResumePrevious,
    Presentation(VideoRenderProfile, Box<[u32]>),
    Audio(Box<dyn AudioBackend>, u32),
    Volume(f32),
}

impl AppliedOperation {
    fn revert(self, session: &mut SessionHandle) -> Result<(), SessionError> {
        match self {
            Self::Runtime(previous) => session.emu.restore(previous),
            Self::ResumePrevious if !session.execution_held => session.emu.core()?.resume()?,
            Self::ResumePrevious => {}
            Self::Presentation(profile, palette) => {
                session
                    .emu
                    .core_mut()?
                    .apply_presentation(profile, palette)?;
            }
            Self::Audio(speaker, rate) => {
                session.emu.core()?.swap_audio_backend(speaker, rate)?;
            }
            Self::Volume(volume) => session.emu.core()?.set_volume(volume)?,
        }
        Ok(())
    }
}

fn rollback(
    session: &mut SessionHandle,
    operations: Vec<AppliedOperation>,
) -> Result<(), SessionError> {
    // A failed inverse must not prevent attempts to undo earlier operations.
    let mut failure = None;
    for operation in operations.into_iter().rev() {
        if let Err(error) = operation.revert(session) {
            failure.get_or_insert(error);
        }
    }
    failure.map_or(Ok(()), Err)
}

fn volume(settings: &SettingsSnapshot) -> f32 {
    if settings.local.audio.muted {
        0.0
    } else {
        f32::from(settings.local.audio.master_volume_percent) / 100.0
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
        // Validate before any live operation, speaker construction, or mapper flush.
        validate_shared_settings(&next_settings.shared)?;
        validate_local_settings(&next_settings.local)?;
        let derived = self.derive_apply(&next_settings);
        // One lock-free tick for the whole apply: the tick threads
        // through the runtime apply and the persistence re-bind below.
        let tick = self.tick_metrics();

        if derived.plan.persistence_changed
            && let Ok(exporter) = self.emu.exporter()
        {
            self.media.persistence.flush_mapper_save(exporter)?;
        }

        let mut applied = Vec::new();
        let result = self
            .apply_live_runtime(&derived, &next_settings, &tick, &mut applied)
            .and_then(|()| {
                self.settings
                    .manager
                    .commit_snapshot(next_settings)
                    .map_err(Into::into)
            });
        let committed = match result {
            Ok(snapshot) => snapshot,
            Err(error) => {
                if let Err(rollback) = rollback(self, applied) {
                    // Never advertise a known-running state after failed compensation.
                    let _ = self
                        .emu
                        .core()
                        .and_then(|core| core.pause().map_err(Into::into));
                    self.emu.invalidate_debug();
                    self.debug_clear_write();
                    return Err(SessionError::Rollback {
                        apply: Box::new(error),
                        rollback: Box::new(rollback),
                    });
                }
                return Err(error);
            }
        };

        let rebuilt = applied
            .iter()
            .any(|op| matches!(op, AppliedOperation::Runtime(_)));
        let bindings_changed = self.settings.snapshot.shared.input != committed.shared.input;
        self.settings.snapshot = committed;
        if derived.assignments_changed
            && let Some(a) = derived.next_assignments
        {
            self.input.current_assignments = a;
        }

        // Rebind only after saving: resolve against the newly committed settings.
        if (derived.plan.persistence_changed || rebuilt)
            && (tick.loaded || self.media.loaded.is_some())
        {
            let restore = if rebuilt && !tick.loaded {
                MapperRestore::Load
            } else {
                MapperRestore::Skip
            };
            self.bind_settings_persistence(restore);
        }
        if rebuilt || derived.assignments_changed || bindings_changed {
            self.rebind_input();
        }
        if rebuilt {
            self.debug_clear_write();
            if tick.loaded && !tick.paused && !self.execution_held {
                self.emu
                    .core()?
                    .resume()
                    .map_err(|_| nerust_emu_thread::OperationError::NoReply)?;
            }
        }
        Ok(derived.plan)
    }

    /// Rebuild the keyboard reverse map after anything that moves
    /// bindings, assignments, or controllers.
    fn rebind_input(&mut self) {
        // Release old fields without losing physical key repeat suppression.
        if let Some(input) = self.emu.input_mut() {
            input.clear();
        }
        self.rebuild_key_field_map();
        for key in &self.input.pressed_keys {
            if let Some(&field) = self.input.key_field_map.get(key)
                && let Some(input) = self.emu.input_mut()
            {
                let _ = input.set(field, nerust_input_traits::InputValue::Digital(true));
            }
        }
        self.input.invalidate_topology();
    }

    pub fn set_fullscreen_default(
        &mut self,
        fullscreen: bool,
    ) -> Result<SettingsApplyPlan, SessionError> {
        if self.settings.snapshot.local.video.window.fullscreen_default == fullscreen {
            return Ok(SettingsApplyPlan::default());
        }

        let mut next_settings = self.settings.snapshot.as_ref().clone();
        next_settings.local.video.window.fullscreen_default = fullscreen;
        self.apply_settings(next_settings)
    }

    /// All fallible preparation completes before replacement; no persistence binding here.
    fn prepare_runtime_for_settings(
        &self,
        next_settings: &SettingsSnapshot,
        assignments: &InputAssignments,
        tick: &TickMetrics,
    ) -> Result<ActiveRuntime, SessionError> {
        let core_bytes = tick
            .loaded
            .then(|| {
                self.emu
                    .core()?
                    .save_state_raw()
                    .map_err(|error| SessionError::from(error.slot_failure()))
            })
            .transpose()?;

        let factory = self.active_factory_cloned().ok_or(SessionError::NoCore)?;
        let rebuilt =
            build_core_for_settings(self.speaker_factory(), next_settings, &factory, assignments)?;
        if let Some(loaded) = self.media.loaded.as_ref() {
            // Preparation must not run guest frames before restoring state.
            rebuilt
                .emu_core
                .load_paused(&loaded.media, Some(loaded.options.clone()))?;
            if let Some(bytes) = core_bytes {
                rebuilt.emu_core.load_state_raw(bytes)?;
            }
            // The worker-side pause-stable adapter keeps state imports parked atomically.
        }
        Ok(ActiveRuntime::from(rebuilt))
    }

    fn bind_settings_persistence(&mut self, restore: MapperRestore) {
        let rom_path = self
            .media
            .loaded
            .as_ref()
            .and_then(|loaded| loaded.media.native_path().map(Path::to_path_buf));
        PersistenceBinder::bind(self, rom_path.as_deref(), restore);
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
        if let Ok(snapshot) = self.settings.manager.snapshot_arc() {
            self.settings.snapshot = snapshot;
        }
    }
}

/// Build a fresh core for `next_settings` plus `assignments`, with no session borrowing.
/// Free function of exactly what the build consumes, so callers hold the session borrow.
pub(super) fn build_core_for_settings(
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
    use std::sync::{Mutex, atomic::AtomicBool};

    use nerust_core_traits::{EmuCommand, identity::SystemId};
    use nerust_input_traits::InputSystemFactory;
    use nerust_render_traits::{FrameBuffer, PixelFormat};
    use nerust_system_traits::factory::{
        CoreParts, FactoryError,
        descriptor::{SystemSettingsChoiceId, SystemSettingsFieldId, SystemSettingsPageModel},
        load::{
            DynSystemLoadOptions, DynSystemLoadOptionsSchema, MediaObject, ResolvedLoadRequest,
        },
        settings::FactorySettingsView,
    };

    use crate::emu_core::{EmuCore, EmuThreadPort};
    use crate::session::settings_apply::*;
    use crate::session::test_util::{test_session, test_session_with_failing_settings};
    use crate::test_helpers::{DummySystemId, MockFactory};

    struct SwapFactory;

    impl CoreFactory for SwapFactory {
        fn system_id(&self) -> Box<dyn SystemId> {
            MockFactory.system_id()
        }
        fn display_name(&self) -> &'static str {
            "settings swap test"
        }
        fn probe_media(&self, _: &MediaObject) -> bool {
            true
        }
        fn settings_page(&self, _: &FactorySettingsView) -> SystemSettingsPageModel {
            unreachable!()
        }
        fn apply_settings_choice(
            &self,
            _: &mut FactorySettingsView,
            _: &SystemSettingsFieldId,
            _: &SystemSettingsChoiceId,
        ) -> Result<(), FactoryError> {
            unreachable!()
        }
        fn resolve_load_request(
            &self,
            _: &FactorySettingsView,
            _: Box<dyn DynSystemLoadOptions>,
        ) -> Result<ResolvedLoadRequest, FactoryError> {
            unreachable!()
        }
        fn default_load_options(&self) -> Box<dyn DynSystemLoadOptions> {
            unreachable!()
        }
        fn load_options_schema(&self) -> Box<dyn DynSystemLoadOptionsSchema> {
            unreachable!()
        }
        fn create_core_and_adapter_with_assignments(
            &self,
            view: &FactorySettingsView,
            speaker: Box<dyn AudioBackend>,
            assignments: &InputAssignments,
        ) -> Result<CoreParts, FactoryError> {
            MockFactory.create_core_and_adapter_with_assignments(view, speaker, assignments)
        }
        fn input_system_factory(&self) -> &dyn InputSystemFactory {
            MockFactory.input_system_factory()
        }
        fn presentation_swap_assets(
            &self,
            _: &FactorySettingsView,
            after: &FactorySettingsView,
        ) -> Option<(VideoRenderProfile, Box<[u32]>)> {
            let parts = crate::test_helpers::build_test_core_parts();
            let mut profile = parts.render_profile;
            profile.logical_size.width = if after.system_config.is_some() {
                602
            } else {
                256
            };
            Some((profile, parts.palette))
        }
    }

    fn visual_delta(session: &mut SessionHandle) -> SettingsSnapshot {
        session.registry = Arc::new(crate::registry::SystemRegistry::new(vec![Arc::new(
            SwapFactory,
        )]));
        let mut next = session.settings_snapshot().clone();
        next.shared.systems.insert(
            Box::new(DummySystemId),
            Box::new(nerust_nes_settings::NesSettings::default()),
        );
        next
    }

    /// Observe real commands at the transport boundary rather than only checking settings cache.
    #[derive(Debug)]
    struct VolumePort {
        volumes: Arc<Mutex<Vec<f32>>>,
        frame: Arc<Mutex<FrameBuffer>>,
        fail_volume: bool,
        fail_audio: bool,
        fail_presentation: bool,
        events: Arc<Mutex<Vec<&'static str>>>,
    }

    impl EmuThreadPort for VolumePort {
        fn send(
            &self,
            command: EmuCommand,
        ) -> Result<(), std::sync::mpsc::TrySendError<EmuCommand>> {
            match command {
                EmuCommand::SetVolume(value) if !self.fail_volume => {
                    self.volumes.lock().unwrap().push(value);
                    self.events.lock().unwrap().push("volume");
                    Ok(())
                }
                EmuCommand::SwapAudio(command) if !self.fail_audio => {
                    self.events.lock().unwrap().push("audio");
                    command.reply.send(()).unwrap();
                    Ok(())
                }
                EmuCommand::SwapFrameSlot(command) if !self.fail_presentation => {
                    self.events.lock().unwrap().push("presentation");
                    command.reply.send(()).unwrap();
                    Ok(())
                }
                other => Err(std::sync::mpsc::TrySendError::Disconnected(other)),
            }
        }
        fn shared_frame_buffer(&self) -> &Arc<Mutex<FrameBuffer>> {
            &self.frame
        }
        fn frame_count(&self) -> u64 {
            0
        }
        fn fps(&self) -> f32 {
            0.0
        }
    }

    fn observe_volume(session: &mut SessionHandle, fail_volume: bool) -> Arc<Mutex<Vec<f32>>> {
        observe_operations(session, fail_volume, false, false).0
    }

    type ObservedOperations = (Arc<Mutex<Vec<f32>>>, Arc<Mutex<Vec<&'static str>>>);

    fn observe_operations(
        session: &mut SessionHandle,
        fail_volume: bool,
        fail_audio: bool,
        fail_presentation: bool,
    ) -> ObservedOperations {
        let parts = crate::test_helpers::build_test_core_parts();
        let volumes = Arc::new(Mutex::new(Vec::new()));
        let events = Arc::new(Mutex::new(Vec::new()));
        let frame = Arc::new(Mutex::new(FrameBuffer::with_capacity(
            0,
            0,
            PixelFormat::Rgba,
        )));
        let core = EmuCore::new(
            Arc::new(VolumePort {
                volumes: volumes.clone(),
                frame: frame.clone(),
                fail_volume,
                fail_audio,
                fail_presentation,
                events: events.clone(),
            }),
            parts.render_profile,
            frame,
            FrameBuffer::with_capacity(0, 0, PixelFormat::Rgba),
            Arc::new(AtomicBool::new(false)),
        );
        session.emu.replace(ActiveRuntime {
            core: Box::new(core),
            input: parts.gui_input,
            field_map: parts.field_map,
            host_peripherals: parts.host_peripherals,
        });
        (volumes, events)
    }

    #[test]
    fn combined_audio_visual_success_keeps_both_changes() {
        let mut session = test_session();
        let (_, events) = observe_operations(&mut session, false, false, false);
        let mut next = visual_delta(&mut session);
        next.local.audio.latency_ms = 90;
        next.local.audio.muted = true;
        let plan = session.apply_settings(next.clone()).unwrap();
        assert!(plan.visual_changed && plan.audio_backend_changed && plan.audio_volume_changed);
        assert_eq!(*events.lock().unwrap(), vec!["presentation", "audio"]);
        assert_eq!(session.render_profile().unwrap().logical_size.width, 602);
        assert_eq!(session.settings_snapshot(), &next);
    }

    #[test]
    fn combined_audio_visual_applies_both_and_undoes_in_reverse_order() {
        let (mut session, dir) = test_session_with_failing_settings();
        let original = session.settings_snapshot().clone();
        let (_, events) = observe_operations(&mut session, false, false, false);
        let mut next = visual_delta(&mut session);
        next.local.audio.latency_ms = 90;
        next.local.audio.muted = true;
        assert!(matches!(
            session.apply_settings(next),
            Err(SessionError::Settings(_))
        ));
        assert_eq!(
            *events.lock().unwrap(),
            vec!["presentation", "audio", "audio", "presentation"]
        );
        assert_eq!(session.render_profile().unwrap().logical_size.width, 256);
        assert_eq!(session.settings_snapshot(), &original);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn later_audio_or_volume_failure_restores_presentation() {
        for audio in [false, true] {
            let mut session = test_session();
            let original = session.settings_snapshot().clone();
            let (_, events) = observe_operations(&mut session, !audio, audio, false);
            let mut next = visual_delta(&mut session);
            if audio {
                next.local.audio.latency_ms = 90;
            } else {
                next.local.audio.muted = true;
            }
            assert!(matches!(
                session.apply_settings(next),
                Err(SessionError::Operation(_))
            ));
            assert_eq!(
                *events.lock().unwrap(),
                vec!["presentation", "presentation"]
            );
            assert_eq!(session.render_profile().unwrap().logical_size.width, 256);
            assert_eq!(session.settings_snapshot(), &original);
        }
    }

    #[test]
    fn presentation_failure_does_not_apply_audio_or_save() {
        let mut session = test_session();
        let original = session.settings_snapshot().clone();
        let (_, events) = observe_operations(&mut session, false, false, true);
        let mut next = visual_delta(&mut session);
        next.local.audio.latency_ms = 90;
        assert!(matches!(
            session.apply_settings(next),
            Err(SessionError::Operation(_))
        ));
        assert!(events.lock().unwrap().is_empty());
        assert_eq!(session.settings.manager.snapshot().unwrap(), original);
    }

    #[test]
    fn volume_and_mute_send_real_commands() {
        let mut session = test_session();
        let volumes = observe_volume(&mut session, false);
        let mut next = session.settings_snapshot().clone();
        next.local.audio.master_volume_percent = 25;
        session.apply_settings(next.clone()).unwrap();
        next.local.audio.muted = true;
        session.apply_settings(next).unwrap();
        assert_eq!(*volumes.lock().unwrap(), vec![0.25, 0.0]);
    }

    #[test]
    fn save_failure_undoes_volume_and_mute() {
        for mute in [false, true] {
            let (mut session, dir) = test_session_with_failing_settings();
            let original = session.settings_snapshot().clone();
            let volumes = observe_volume(&mut session, false);
            let mut next = original.clone();
            next.local.audio.master_volume_percent = 25;
            next.local.audio.muted = mute;
            assert!(matches!(
                session.apply_settings(next),
                Err(SessionError::Settings(_))
            ));
            assert_eq!(
                *volumes.lock().unwrap(),
                vec![if mute { 0.0 } else { 0.25 }, volume(&original)]
            );
            assert_eq!(session.settings_snapshot(), &original);
            let _ = std::fs::remove_dir_all(dir);
        }
    }

    #[test]
    fn invalid_settings_never_send_volume() {
        let mut session = test_session();
        let volumes = observe_volume(&mut session, false);
        let original = session.settings_snapshot().clone();
        let mut next = original.clone();
        next.local.audio.master_volume_percent = 101;
        next.local.audio.sample_rate = 0;
        assert!(matches!(
            session.apply_settings(next),
            Err(SessionError::Settings(_))
        ));
        assert!(volumes.lock().unwrap().is_empty());
        assert_eq!(session.settings_snapshot(), &original);
    }

    #[test]
    fn volume_failure_is_not_saved() {
        let mut session = test_session();
        observe_volume(&mut session, true);
        let original = session.settings_snapshot().clone();
        let mut next = original.clone();
        next.local.audio.muted = !next.local.audio.muted;
        assert!(matches!(
            session.apply_settings(next),
            Err(SessionError::Operation(_))
        ));
        assert_eq!(session.settings_snapshot(), &original);
        assert_eq!(session.settings.manager.snapshot().unwrap(), original);
    }

    #[test]
    fn save_failure_restores_identical_runtime_and_assignments() {
        let (mut session, dir) = test_session_with_failing_settings();
        let original = session.settings_snapshot().clone();
        let assignments = session.input.current_assignments.clone();
        let core = session.emu.core().unwrap() as *const dyn crate::emu_core::LiveCore as *const ();
        let mut next = original.clone();
        next.app_state.controller_assignments.insert(
            session.active_system_id_box().unwrap(),
            vec![(
                crate::test_helpers::TEST_SLOT_P1.to_string(),
                Some("test.profile.p1".into()),
            )],
        );
        assert!(matches!(
            session.apply_settings(next),
            Err(SessionError::Settings(_))
        ));
        assert_eq!(
            session.emu.core().unwrap() as *const dyn crate::emu_core::LiveCore as *const (),
            core
        );
        assert_eq!(session.input.current_assignments, assignments);
        assert_eq!(session.settings_snapshot(), &original);
        let _ = std::fs::remove_dir_all(dir);
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
