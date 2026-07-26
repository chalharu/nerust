
use std::sync::{Arc, atomic::Ordering::AcqRel};

use nerust_core_traits::{
    audio::AudioBackend,
    factory::{
        CoreFactory, CoreParts, FactoryError,
        descriptor::{SystemSettingsChoiceId, SystemSettingsFieldId, SystemSettingsPageModel},
        load::{DynSystemLoadOptions, MediaObject, ResolvedLoadRequest},
        settings::FactorySettingsView,
    },
    identity::SystemId,
};
use nerust_gui_runtime::settings::{
    HostBackendCapabilities, SettingsApplyPlan, SettingsManager, SettingsSnapshot,
};
use nerust_input_traits::{InputAssignments, InputSystemFactory};

use super::test_util::*;
use crate::test_helpers::*;
use crate::{
    load::{RomLoadTarget, SystemActivationError},
    registry::SystemRegistry,
    session::{
        KeyboardShortcut, SessionError, SessionHandle,
        commands::{SessionCommand, SessionCommandOutcome},
    },
    settings::factory::settings_view,
};

/// Factory that fails on first `create_core_and_adapter_with_assignments`
/// call, then delegates to the inner factory for the fallback path.
struct FailingOnceFactory<T: CoreFactory> {
    inner: Arc<T>,
    has_failed: std::sync::atomic::AtomicBool,
}

impl<T: CoreFactory> FailingOnceFactory<T> {
    fn new(inner: Arc<T>) -> Self {
        Self {
            inner,
            has_failed: std::sync::atomic::AtomicBool::new(false),
        }
    }
}

impl<T: CoreFactory> CoreFactory for FailingOnceFactory<T> {
    fn system_id(&self) -> Box<dyn SystemId> {
        self.inner.system_id()
    }
    fn display_name(&self) -> &'static str {
        self.inner.display_name()
    }
    fn create_core_and_adapter_with_assignments(
        &self,
        view: &FactorySettingsView,
        speaker: Box<dyn AudioBackend>,
        assignments: &InputAssignments,
    ) -> Result<CoreParts, FactoryError> {
        if !self.has_failed.swap(true, AcqRel) {
            return Err(FactoryError::Create("simulated failure".into()));
        }
        self.inner
            .create_core_and_adapter_with_assignments(view, speaker, assignments)
    }
    fn create_core_and_adapter(
        &self,
        view: &FactorySettingsView,
        speaker: Box<dyn AudioBackend>,
    ) -> Result<CoreParts, FactoryError> {
        self.inner.create_core_and_adapter(view, speaker)
    }
    fn probe_media(&self, _media: &MediaObject) -> bool {
        unreachable!()
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
    fn input_system_factory(&self) -> &dyn InputSystemFactory {
        self.inner.input_system_factory()
    }

    fn load_options_schema(
        &self,
    ) -> Box<dyn nerust_core_traits::factory::load::DynSystemLoadOptionsSchema> {
        // CLI parsing not exercised in this test path
        unreachable!()
    }
}

#[test]
fn shortcut_key_returns_shortcut_action_without_controller_event() {
    let mut session = test_session();
    assert_eq!(
        session.handle_keyboard_key(nerust_keyboard::Key::Space, true),
        Some(KeyboardShortcut::Session(
            nerust_gui_settings::input::ShortcutAction::TogglePause
        )),
    );
    assert_eq!(
        session.handle_keyboard_key(nerust_keyboard::Key::Space, true),
        None
    );
}

#[test]
fn system_load_options_flow_into_session_load() {
    let mut session = test_session();
    let resolved = session
        .factory()
        .expect("no active system")
        .resolve_load_request(
            &test_view(&session),
            session
                .factory()
                .expect("no active system")
                .default_load_options(),
        )
        .unwrap();
    assert!(
        session
            .load_resolved(MediaObject::new(None, test_rom()), resolved)
            .is_ok()
    );
}

#[test]
fn session_commands_drive_pause_resume_toggle_and_reset() {
    let mut session = test_session();
    let resolved = session
        .factory()
        .unwrap()
        .resolve_load_request(&test_view(&session), NoopSystemLoadOptions.into())
        .unwrap();
    session
        .load_resolved(MediaObject::new(None, test_rom()), resolved)
        .unwrap();

    assert!(session.can_resume());
    assert_eq!(
        session.run_command(SessionCommand::Resume).unwrap(),
        SessionCommandOutcome {
            executed: true,
            needs_redraw: true,
        }
    );
    assert!(session.can_pause());
    assert_eq!(
        session.run_command(SessionCommand::Pause).unwrap(),
        SessionCommandOutcome {
            executed: true,
            needs_redraw: false,
        }
    );
    assert_eq!(
        session.run_command(SessionCommand::Pause).unwrap(),
        SessionCommandOutcome::default()
    );
    assert!(
        session
            .run_command(SessionCommand::TogglePause)
            .unwrap()
            .executed
    );
    assert!(session.run_command(SessionCommand::Reset).unwrap().executed);
}

#[test]
fn session_commands_report_missing_core_and_empty_slots() {
    let registry = Arc::new(SystemRegistry::new(vec![Arc::new(MockFactory)]));
    let audio_registry = Arc::new(nerust_core_traits::audio::AudioBackendRegistry::new());
    let mut session = SessionHandle::new_ephemeral(test_capabilities(), registry, audio_registry);

    assert!(matches!(
        session.run_command(SessionCommand::Reset),
        Err(SessionError::NoCore)
    ));
    assert_eq!(
        session.run_command(SessionCommand::LoadActiveSlot).unwrap(),
        SessionCommandOutcome::default()
    );
    assert_eq!(
        session.run_command(SessionCommand::SelectNextSlot).unwrap(),
        SessionCommandOutcome::default()
    );
    assert!(!session.save_hidden_lifecycle_state());
    assert!(!session.load_hidden_lifecycle_state());
}

#[test]
fn session_rebuild_reuses_previously_resolved_load_request() {
    let mut session = test_session();
    let options = session
        .factory()
        .expect("no active system")
        .default_load_options();
    let resolved = session
        .factory()
        .expect("no active system")
        .resolve_load_request(&test_view(&session), options)
        .unwrap();
    session
        .load_resolved(MediaObject::new(None, test_rom()), resolved)
        .unwrap();
    assert!(session.loaded());

    let mut next = session.settings_snapshot().clone();
    next.local.audio.latency_ms = 90;
    let plan = session.apply_settings(next).unwrap();

    assert!(plan.session_rebuild_required);
    assert!(session.loaded());
}

#[test]
fn apply_settings_skips_rebuild_when_assignments_unchanged() {
    let mut session = test_session();
    let creations_before =
        crate::test_helpers::CORE_CREATION_COUNT.load(std::sync::atomic::Ordering::Relaxed);

    let snapshot = session.settings_snapshot().clone();
    let plan = session.apply_settings(snapshot).unwrap();

    assert!(!plan.session_rebuild_required);
    let creations_after =
        crate::test_helpers::CORE_CREATION_COUNT.load(std::sync::atomic::Ordering::Relaxed);
    assert_eq!(
        creations_after, creations_before,
        "no core creation should occur when no rebuild needed"
    );
}

#[test]
fn apply_settings_rebuilds_when_latency_changes() {
    let mut session = test_session();
    let creations_before =
        crate::test_helpers::CORE_CREATION_COUNT.load(std::sync::atomic::Ordering::Relaxed);
    let mut next = session.settings_snapshot().clone();
    next.local.audio.latency_ms = 90;

    let plan = session.apply_settings(next).unwrap();
    assert!(
        plan.session_rebuild_required,
        "latency change should require rebuild"
    );
    let creations_after =
        crate::test_helpers::CORE_CREATION_COUNT.load(std::sync::atomic::Ordering::Relaxed);
    assert!(
        creations_after > creations_before,
        "core should have been rebuilt"
    );
}

#[test]
fn apply_settings_rolls_back_on_save_failure() {
    use crate::settings::defaults::seed::{
        default_app_state, default_local_settings, default_shared_settings,
    };
    use nerust_gui_runtime::settings::repository::FailingStore;

    let mut session = test_session();
    let original = session.settings_snapshot().clone();
    let original_assignments = session.current_assignments.clone();
    let mut modified = original.clone();
    modified.local.audio.latency_ms = 90;

    // Replace settings manager with one using a FailingStore
    let registry = crate::registry::SystemRegistry::new(vec![]);
    let shared = default_shared_settings(registry.all());
    session.settings = SettingsManager::with_store(
        shared,
        default_local_settings(),
        default_app_state(),
        Box::new(FailingStore),
    );

    // Save fails → rollback should restore core state
    let err = session.apply_settings(modified).unwrap_err();
    assert!(
        matches!(err, SessionError::Settings(_)),
        "expected Settings error, got {err:?}"
    );

    // Snapshot should be rolled back to original
    assert_eq!(
        session.settings_snapshot().local.audio.latency_ms,
        original.local.audio.latency_ms,
        "snapshot should be rolled back on save failure"
    );

    // Assignments should be rolled back
    assert_eq!(
        session.current_assignments.to_string_pairs(),
        original_assignments.to_string_pairs(),
        "assignments should be rolled back on save failure"
    );

    // Core should still be present (rollback rebuild succeeded)
    assert!(
        session.emu_core.is_some(),
        "core should be restored after rollback"
    );
}

#[test]
fn apply_settings_rebuilds_when_assignments_change() {
    use crate::test_helpers::TEST_SLOT_P1;
    let mut session = test_session();
    let creations_before =
        crate::test_helpers::CORE_CREATION_COUNT.load(std::sync::atomic::Ordering::Relaxed);
    let prev_pairs = session.current_assignments.to_string_pairs();

    let sid = session.factory().unwrap().system_id();
    let mut next = session.settings_snapshot().clone();
    next.app_state.controller_assignments.insert(
        sid,
        vec![(
            TEST_SLOT_P1.to_string(),
            Some("test.profile.p1".to_string()),
        )],
    );

    let _ = session.apply_settings(next).unwrap();

    let new_pairs = session.current_assignments.to_string_pairs();
    assert_ne!(
        new_pairs, prev_pairs,
        "assignments should be updated after apply"
    );
    assert!(
        new_pairs
            .iter()
            .any(|(slot, _)| slot == TEST_SLOT_P1.as_str()),
        "new assignments should include the test slot"
    );
    let creations_after =
        crate::test_helpers::CORE_CREATION_COUNT.load(std::sync::atomic::Ordering::Relaxed);
    assert!(
        creations_after > creations_before,
        "core should have been rebuilt with new assignments"
    );
}

#[test]
fn set_fullscreen_default_updates_snapshot_and_plan() {
    let mut session = test_session();
    session.handle_keyboard_key(nerust_keyboard::Key::KeyZ, true);
    let plan = session
        .set_fullscreen_default(true)
        .expect("set_fullscreen_default should succeed");
    assert_eq!(
        plan,
        SettingsApplyPlan {
            window_settings_changed: true,
            fullscreen_default_changed: true,
            ..SettingsApplyPlan::default()
        }
    );
    assert!(
        session
            .settings_snapshot()
            .local
            .video
            .window
            .fullscreen_default
    );
    let second = session
        .set_fullscreen_default(true)
        .expect("second set_fullscreen_default should succeed");
    assert_eq!(second, SettingsApplyPlan::default());
}

#[test]
fn session_creation_falls_back_to_defaults_when_custom_settings_fail() {
    let failing = Arc::new(FailingOnceFactory::new(Arc::new(MockFactory)));
    let system_id = failing.system_id();
    let expected_assignments = failing.input_system_factory().default_assignments();
    let registry = Arc::new(SystemRegistry::new(vec![failing]));
    let audio_registry = Arc::new(nerust_core_traits::audio::AudioBackendRegistry::new());
    let capabilities = nerust_gui_runtime::settings::HostBackendCapabilities {
        window: nerust_gui_runtime::settings::HostWindowCapabilities {
            remembers_window_size: false,
            supports_fullscreen_default: true,
            supports_scaling: true,
        },
        presentation: None,
    };
    let mut session = SessionHandle::new(capabilities, registry, audio_registry)
        .expect("session creation should succeed even with failing factory");
    session
        .settings_snapshot
        .app_state
        .controller_assignments
        .insert(
            system_id.clone(),
            vec![
                ("nes.attachment.player1".to_string(), None),
                ("nes.attachment.player2".to_string(), None),
            ],
        );
    RomLoadTarget::set_active_system(&mut session, system_id.as_ref())
        .expect("fallback core creation should succeed");
    assert_eq!(
        session.current_assignments.to_string_pairs(),
        expected_assignments.to_string_pairs()
    );
}

#[test]
fn session_factory_uses_primary_initially() {
    let factory = Arc::new(MockFactory);
    let id = factory.system_id();
    let registry = Arc::new(SystemRegistry::new(vec![factory]));
    let audio_registry = Arc::new(nerust_core_traits::audio::AudioBackendRegistry::new());
    let mut session = SessionHandle::new_ephemeral(test_capabilities(), registry, audio_registry);
    assert!(session.factory().is_none());
    RomLoadTarget::set_active_system(&mut session, id.as_ref())
        .expect("test setup should succeed for known system");
    assert_eq!(session.factory().expect("no active system").system_id(), id);
}

#[test]
fn set_active_system_rejects_unknown_id() {
    let factory = Arc::new(MockFactory);
    let registry = Arc::new(SystemRegistry::new(vec![factory]));
    let audio_registry = Arc::new(nerust_core_traits::audio::AudioBackendRegistry::new());
    let mut session = SessionHandle::new_ephemeral(test_capabilities(), registry, audio_registry);

    let err = RomLoadTarget::set_active_system(&mut session, &DummyOtherSystemId).unwrap_err();
    assert!(matches!(err, SystemActivationError::NotRegistered(_)));
    assert!(session.active_system_id().is_none());
    assert!(session.factory().is_none());
}

#[test]
fn switching_system_discards_previous_loaded_runtime_state() {
    let first: Arc<dyn CoreFactory> = Arc::new(MockFactory);
    let second: Arc<dyn CoreFactory> = Arc::new(AlternateMockFactory);
    let second_id = second.system_id();
    let registry = Arc::new(SystemRegistry::new(vec![first.clone(), second]));
    let audio_registry = Arc::new(nerust_core_traits::audio::AudioBackendRegistry::new());
    let mut session = SessionHandle::new_ephemeral(test_capabilities(), registry, audio_registry);

    RomLoadTarget::set_active_system(&mut session, first.system_id().as_ref()).unwrap();
    let resolved = session
        .factory()
        .unwrap()
        .resolve_load_request(&test_view(&session), NoopSystemLoadOptions.into())
        .unwrap();
    session
        .load_resolved(MediaObject::new(None, test_rom()), resolved)
        .unwrap();
    assert!(session.loaded());

    RomLoadTarget::set_active_system(&mut session, second_id.as_ref()).unwrap();

    assert_eq!(session.active_system_id(), Some(second_id.as_ref()));
    assert!(!session.loaded());
    assert!(session.loaded_media.is_none());
    assert!(session.slots().is_empty());
}

fn test_capabilities() -> HostBackendCapabilities {
    HostBackendCapabilities {
        window: nerust_gui_runtime::settings::HostWindowCapabilities {
            remembers_window_size: false,
            supports_fullscreen_default: false,
            supports_scaling: false,
        },
        presentation: None,
    }
}

#[test]
fn registry_all_produces_settings_page_for_registered_system() {
    use crate::settings::defaults::seed::{
        default_app_state, default_local_settings, default_shared_settings,
    };
    let factory = Arc::new(MockFactory);
    let registry = SystemRegistry::new(vec![factory]);
    let snapshot = SettingsSnapshot {
        shared: default_shared_settings(&[]),
        local: default_local_settings(),
        app_state: default_app_state(),
    };
    let pages: Vec<_> = registry
        .all()
        .iter()
        .map(|f| {
            let view = settings_view(&snapshot, f.system_id().as_ref());
            f.settings_page(&view)
        })
        .collect();
    assert_eq!(
        pages.len(),
        1,
        "should produce one page per registered system"
    );
}
