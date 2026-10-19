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
use nerust_gui_viewmodel::debugger::{PendingWrite, StatusMessage};
use nerust_input_traits::{InputAssignments, InputSystemFactory};
use nerust_settings_core::factory::settings_view;
use nerust_settings_core::model::language::AppLanguage;

use super::test_util::*;
use crate::test_helpers::*;
use crate::{
    load::{RomLoadTarget, SystemActivationError},
    registry::SystemRegistry,
    session::{
        KeyboardShortcut, SessionError, SessionHandle,
        commands::{SessionCommand, SessionCommandOutcome},
        error::SlotOpFailure,
    },
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
            nerust_settings_core::model::input::ShortcutAction::TogglePause
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

    assert!(session.can_pause());
    assert!(!session.can_resume());
    assert_eq!(
        session.run_command(SessionCommand::Pause).unwrap(),
        SessionCommandOutcome {
            executed: true,
            needs_redraw: false,
            slot_failure: None,
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
    assert!(session.can_pause());
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
        SessionCommandOutcome {
            executed: false,
            needs_redraw: false,
            slot_failure: Some(SlotOpFailure::Empty),
        }
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

/// `MockFactory` delegate answering the presentation query.
/// Direction-aware by entry presence (absent→present wide,
/// present→absent narrow); pins the session's `(before, after)` order.
struct LiveSwapFactory;
impl CoreFactory for LiveSwapFactory {
    fn system_id(&self) -> Box<dyn SystemId> {
        MockFactory.system_id()
    }
    fn display_name(&self) -> &'static str {
        MockFactory.display_name()
    }
    fn probe_media(&self, media: &MediaObject) -> bool {
        MockFactory.probe_media(media)
    }
    fn settings_page(&self, view: &FactorySettingsView) -> SystemSettingsPageModel {
        MockFactory.settings_page(view)
    }
    fn apply_settings_choice(
        &self,
        view: &mut FactorySettingsView,
        field: &SystemSettingsFieldId,
        choice: &SystemSettingsChoiceId,
    ) -> Result<(), FactoryError> {
        MockFactory.apply_settings_choice(view, field, choice)
    }
    fn resolve_load_request(
        &self,
        view: &FactorySettingsView,
        options: Box<dyn DynSystemLoadOptions>,
    ) -> Result<ResolvedLoadRequest, FactoryError> {
        MockFactory.resolve_load_request(view, options)
    }
    fn default_load_options(&self) -> Box<dyn DynSystemLoadOptions> {
        MockFactory.default_load_options()
    }
    fn load_options_schema(
        &self,
    ) -> Box<dyn nerust_core_traits::factory::load::DynSystemLoadOptionsSchema> {
        MockFactory.load_options_schema()
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
        before: &FactorySettingsView,
        after: &FactorySettingsView,
    ) -> Option<(nerust_render_traits::VideoRenderProfile, Box<[u32]>)> {
        use nerust_render_traits::{
            VideoFrameFormat, VideoRenderProfile, logical::LogicalSize, physical::PhysicalSize,
        };
        let widened = after.system_config.is_some() && before.system_config.is_none();
        let restored = before.system_config.is_some() && after.system_config.is_none();
        let width = if widened {
            602
        } else if restored {
            256
        } else {
            return None;
        };
        Some((
            VideoRenderProfile {
                source_logical_size: LogicalSize {
                    width: 256,
                    height: 240,
                },
                logical_size: LogicalSize { width, height: 240 },
                physical_size: PhysicalSize {
                    width: width as f32,
                    height: 480.0,
                },
                frame_format: VideoFrameFormat::Palette,
                ntsc_packed_rgba8: None,
            },
            (0..256).map(|i| 0xFF00_0000 | i).collect(),
        ))
    }
}

/// Loaded session behind `LiveSwapFactory` for the presentation-swap
/// tests below.
fn live_swap_session() -> SessionHandle {
    let factory: Arc<dyn CoreFactory> = Arc::new(LiveSwapFactory);
    let audio_registry = Arc::new(nerust_core_traits::audio::AudioBackendRegistry::new());
    let registry = Arc::new(SystemRegistry::new(vec![factory.clone()]));
    let mut session = SessionHandle::new_ephemeral(test_capabilities(), registry, audio_registry);
    session
        .set_active_system(factory.system_id().as_ref())
        .unwrap();
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
    session
        .load_resolved(MediaObject::new(None, test_rom()), resolved)
        .unwrap();
    assert!(session.loaded());
    session
}

/// Visual delta used by the presentation-swap tests: the active
/// system gains settings it didn't have (absent → present).
fn with_visual_delta(session: &SessionHandle) -> SettingsSnapshot {
    let mut next = session.settings_snapshot().clone();
    let mut nes = nerust_nes_settings::NesSettings::default();
    nes.video.filter = nerust_nes_settings::NesVideoFilter::NtscRgb;
    next.shared
        .systems
        .insert(Box::new(DummySystemId) as Box<dyn SystemId>, Box::new(nes));
    next
}

#[test]
fn apply_settings_swaps_presentation_without_rebuild_on_visual_delta() {
    use crate::test_helpers::mock_core_creations;

    let mut session = live_swap_session();
    let next = with_visual_delta(&session);

    let creations = mock_core_creations();
    let plan = session.apply_settings(next).unwrap();
    assert!(
        plan.session_rebuild_required,
        "visual delta still flags the plan"
    );
    assert_eq!(
        mock_core_creations(),
        creations,
        "presentation swap must not reconstruct the core"
    );
    assert!(session.loaded(), "swap keeps the loaded ROM");
    assert_eq!(
        session
            .render_profile()
            .expect("profile")
            .logical_size
            .width,
        602,
        "swapped profile reaches the session"
    );
}

#[test]
fn apply_settings_rolls_back_presentation_swap_on_save_failure() {
    use crate::settings::defaults::seed::{
        default_app_state, default_local_settings, default_shared_settings,
    };
    use nerust_gui_runtime::settings::repository::FailingStore;

    let mut session = live_swap_session();
    let next = with_visual_delta(&session);

    // Replace settings manager with one using a FailingStore.
    let registry = crate::registry::SystemRegistry::new(vec![]);
    let shared = default_shared_settings(registry.all());
    session.set_settings(SettingsManager::with_store(
        shared,
        default_local_settings(),
        default_app_state(),
        Box::new(FailingStore),
    ));

    // Save fails after the swap: the error surfaces and the previous
    // assets swap back behind a fresh reply (narrow profile again).
    let err = session.apply_settings(next).unwrap_err();
    assert!(
        matches!(err, SessionError::Settings(_)),
        "expected Settings error, got {err:?}"
    );
    assert_eq!(
        session
            .render_profile()
            .expect("profile")
            .logical_size
            .width,
        256,
        "rollback swaps the previous assets back"
    );
    assert!(
        !session
            .settings_snapshot()
            .shared
            .systems
            .contains_key(&DummySystemId as &dyn SystemId),
        "snapshot keeps the previous (absent) settings"
    );
    assert!(session.loaded(), "rollback keeps the loaded ROM");
}

#[test]
fn apply_settings_skips_rebuild_when_assignments_unchanged() {
    let mut session = test_session();
    let snapshot = session.settings_snapshot().clone();
    let plan = session.apply_settings(snapshot).unwrap();
    assert!(!plan.session_rebuild_required);
}

#[test]
fn apply_settings_swaps_audio_without_rebuild_when_latency_changes() {
    use crate::test_helpers::mock_core_creations;

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

    let creations = mock_core_creations();
    let plan = session.apply_settings(next).unwrap();
    assert!(
        plan.audio_backend_changed,
        "latency change should swap the backend"
    );
    assert_eq!(
        mock_core_creations(),
        creations,
        "live swap must not reconstruct the core"
    );
    assert!(session.loaded(), "swap keeps the loaded ROM");
    assert_eq!(session.settings_snapshot().local.audio.latency_ms, 90);
}

#[test]
fn apply_settings_rolls_back_on_save_failure() {
    use crate::settings::defaults::seed::{
        default_app_state, default_local_settings, default_shared_settings,
    };
    use nerust_gui_runtime::settings::repository::FailingStore;

    let mut session = test_session();
    let original = session.settings_snapshot().clone();
    let original_assignments = session.input.current_assignments.clone();
    let mut modified = original.clone();
    modified.local.audio.latency_ms = 90;

    // Replace settings manager with one using a FailingStore
    let registry = crate::registry::SystemRegistry::new(vec![]);
    let shared = default_shared_settings(registry.all());
    session.set_settings(SettingsManager::with_store(
        shared,
        default_local_settings(),
        default_app_state(),
        Box::new(FailingStore),
    ));

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
        session.input.current_assignments.to_string_pairs(),
        original_assignments.to_string_pairs(),
        "assignments should be rolled back on save failure"
    );

    // Core should still be present (swap-back rollback succeeded without
    // reconstructing anything)
    assert!(
        session.emu.core_opt().is_some(),
        "core should be restored after rollback"
    );
}

#[test]
fn apply_settings_rebuilds_when_assignments_change() {
    use crate::test_helpers::{TEST_SLOT_P1, mock_core_creations};
    let mut session = test_session();
    let prev_pairs = session.input.current_assignments.to_string_pairs();

    let sid = session.factory().unwrap().system_id();
    let mut next = session.settings_snapshot().clone();
    next.app_state.controller_assignments.insert(
        sid,
        vec![(
            TEST_SLOT_P1.to_string(),
            Some("test.profile.p1".to_string()),
        )],
    );

    let creations = mock_core_creations();
    let _ = session.apply_settings(next).unwrap();
    assert_eq!(
        mock_core_creations(),
        creations + 1,
        "reassignment still reconstructs the core"
    );

    let new_pairs = session.input.current_assignments.to_string_pairs();
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
    let mut snapshot = session.settings_snapshot().clone();
    snapshot.app_state.controller_assignments.insert(
        system_id.clone(),
        vec![
            ("nes.attachment.player1".to_string(), None),
            ("nes.attachment.player2".to_string(), None),
        ],
    );
    session.set_settings_snapshot(snapshot);
    RomLoadTarget::set_active_system(&mut session, system_id.as_ref())
        .expect("fallback core creation should succeed");
    assert_eq!(
        session.input.current_assignments.to_string_pairs(),
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
    assert!(session.loaded_media_ref().is_none());
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

#[test]
fn debug_snapshot_without_core_is_unavailable() {
    let registry = Arc::new(SystemRegistry::new(vec![Arc::new(MockFactory)]));
    let audio_registry = Arc::new(nerust_core_traits::audio::AudioBackendRegistry::new());
    let session = SessionHandle::new_ephemeral(test_capabilities(), registry, audio_registry);
    // No core: paused reads as true (nothing runs), spaces are empty,
    // and the snapshot carries degenerates only.
    assert!(session.debug_paused());
    assert!(session.debug_spaces().is_empty());
    let snapshot = session.debug_snapshot(None, 0, None, false, None);
    assert!(!snapshot.available);
    assert!(snapshot.dump_rows.is_empty());
    assert!(snapshot.disasm_lines.is_empty());
    assert!(
        session
            .debug_read_byte(nerust_core_traits::debugger::SpaceId(0), 0)
            .is_none()
    );
}

#[test]
fn debug_write_transaction_without_core() {
    use nerust_core_traits::debugger::SpaceId;
    let registry = Arc::new(SystemRegistry::new(vec![Arc::new(MockFactory)]));
    let audio_registry = Arc::new(nerust_core_traits::audio::AudioBackendRegistry::new());
    let session = SessionHandle::new_ephemeral(test_capabilities(), registry, audio_registry);
    // Prepare reads nothing without a core, but still arms.
    assert_eq!(session.debug_prepare_write(SpaceId(0), 0x10), None);
    assert_eq!(session.debug_pending_text(), None);
    session.debug_stage_write(0x42);
    assert_eq!(
        session.debug_pending_text(),
        Some(PendingWrite {
            addr: 0x10,
            old: None,
            value: 0x42,
        })
    );
    // Commit without a core reports loudly and consumes.
    assert_eq!(
        session.debug_commit_write(),
        StatusMessage::NoCore,
        "rendered: {}",
        StatusMessage::NoCore.render(AppLanguage::English)
    );
    assert_eq!(session.debug_pending_text(), None);
    assert_eq!(session.debug_commit_write(), StatusMessage::NothingToWrite);
}

#[test]
fn debug_traffic_kill_flows_through_session() {
    use nerust_core_traits::debugger::SpaceId;
    let registry = Arc::new(SystemRegistry::new(vec![Arc::new(MockFactory)]));
    let audio_registry = Arc::new(nerust_core_traits::audio::AudioBackendRegistry::new());
    let session = SessionHandle::new_ephemeral(test_capabilities(), registry, audio_registry);
    session.debug_prepare_write(SpaceId(0), 0x10);
    session.debug_stage_write(0x42);
    // Non-state-changing traffic keeps the staged value.
    session.debug_note_traffic(false);
    assert!(session.debug_pending_text().is_some());
    // State-changing traffic kills it.
    session.debug_note_traffic(true);
    assert_eq!(session.debug_pending_text(), None);
    // Explicit cancel clears too.
    session.debug_prepare_write(SpaceId(0), 0x10);
    session.debug_clear_write();
    assert_eq!(session.debug_pending_text(), None);
}

#[test]
fn debug_drain_select_then_stage_yields_pending() {
    use crate::debug::{DebugAction, DebugDrainInput};
    use nerust_core_traits::debugger::SpaceId;
    let registry = Arc::new(SystemRegistry::new(vec![Arc::new(MockFactory)]));
    let audio_registry = Arc::new(nerust_core_traits::audio::AudioBackendRegistry::new());
    let session = SessionHandle::new_ephemeral(test_capabilities(), registry, audio_registry);
    let input = DebugDrainInput {
        space: Some(SpaceId(0)),
        mem_addr: 0,
        dis_addr: None,
        watch: None,
        freeze: None,
        reg_target: None,
        prev_dump_rows: &[],
        need_images: false,
    };
    // SelectRow prepares fresh state before EditStage builds on it
    // within the same batch, so Write works right after a cancel.
    let output = session.debug_drain(
        &[DebugAction::SelectRow(0x10), DebugAction::EditStage(0x42)],
        &input,
    );
    assert_eq!(output.reselected, Some((0x10, None)));
    assert_eq!(
        output.display.pending,
        Some(PendingWrite {
            addr: 0x10,
            old: None,
            value: 0x42,
        })
    );
    // No execution outcome: status falls back to the pause state.
    assert_eq!(output.display.status, StatusMessage::Paused);
    assert!(output.display.images.is_empty());
}

#[test]
fn debug_drain_kills_stale_values_before_prepare() {
    use crate::debug::{DebugAction, DebugDrainInput};
    use nerust_core_traits::debugger::SpaceId;
    let registry = Arc::new(SystemRegistry::new(vec![Arc::new(MockFactory)]));
    let audio_registry = Arc::new(nerust_core_traits::audio::AudioBackendRegistry::new());
    let session = SessionHandle::new_ephemeral(test_capabilities(), registry, audio_registry);
    let input = DebugDrainInput {
        space: Some(SpaceId(0)),
        mem_addr: 0,
        dis_addr: None,
        watch: None,
        freeze: None,
        reg_target: None,
        prev_dump_rows: &[],
        need_images: false,
    };
    let staged = session.debug_drain(
        &[DebugAction::SelectRow(0x10), DebugAction::EditStage(0x42)],
        &input,
    );
    assert!(staged.display.pending.is_some());
    // A state-changing batch kills the staged value from the
    // previous batch (kill runs before prepare inside the drain).
    let killed = session.debug_drain(&[DebugAction::Refresh], &input);
    assert_eq!(killed.display.pending, None);
    assert_eq!(killed.reselected, None);
}

#[test]
fn debug_drain_cancel_and_commit_consume() {
    use crate::debug::{DebugAction, DebugDrainInput};
    use nerust_core_traits::debugger::SpaceId;
    let registry = Arc::new(SystemRegistry::new(vec![Arc::new(MockFactory)]));
    let audio_registry = Arc::new(nerust_core_traits::audio::AudioBackendRegistry::new());
    let session = SessionHandle::new_ephemeral(test_capabilities(), registry, audio_registry);
    let input = DebugDrainInput {
        space: Some(SpaceId(0)),
        mem_addr: 0,
        dis_addr: None,
        watch: None,
        freeze: None,
        reg_target: None,
        prev_dump_rows: &[],
        need_images: false,
    };
    session.debug_drain(
        &[DebugAction::SelectRow(0x10), DebugAction::EditStage(0x42)],
        &input,
    );
    let cancelled = session.debug_drain(&[DebugAction::EditCancel], &input);
    assert_eq!(cancelled.display.pending, None);
    session.debug_drain(
        &[DebugAction::SelectRow(0x10), DebugAction::EditStage(0x42)],
        &input,
    );
    // Commit without a core reports loudly and consumes.
    let committed = session.debug_drain(&[DebugAction::WriteConfirm], &input);
    assert_eq!(committed.display.status, StatusMessage::NoCore);
    assert_eq!(committed.display.pending, None);
}

#[test]
fn debug_drain_propagates_execution_outcomes() {
    use crate::debug::{DebugAction, DebugDrainInput};
    let registry = Arc::new(SystemRegistry::new(vec![Arc::new(MockFactory)]));
    let audio_registry = Arc::new(nerust_core_traits::audio::AudioBackendRegistry::new());
    let session = SessionHandle::new_ephemeral(test_capabilities(), registry, audio_registry);
    let input = DebugDrainInput {
        space: None,
        mem_addr: 0,
        dis_addr: None,
        watch: None,
        freeze: None,
        reg_target: None,
        prev_dump_rows: &[],
        need_images: false,
    };
    // Failures stay loud instead of degrading into idle text.
    let stepped = session.debug_drain(&[DebugAction::StepFrame], &input);
    assert_eq!(stepped.display.status, StatusMessage::NoCore);
}

#[test]
fn debug_control_without_core_reports_loudly() {
    let registry = Arc::new(SystemRegistry::new(vec![Arc::new(MockFactory)]));
    let audio_registry = Arc::new(nerust_core_traits::audio::AudioBackendRegistry::new());
    let session = SessionHandle::new_ephemeral(test_capabilities(), registry, audio_registry);
    assert_eq!(session.debug_pause(), StatusMessage::NoCore);
    assert_eq!(session.debug_resume(), StatusMessage::NoCore);
    assert_eq!(
        session.debug_step(nerust_core_traits::debugger::StepUnit::Frame),
        StatusMessage::NoCore
    );
}

#[test]
fn debug_write_byte_without_core_is_silent() {
    use nerust_core_traits::debugger::SpaceId;
    let registry = Arc::new(SystemRegistry::new(vec![Arc::new(MockFactory)]));
    let audio_registry = Arc::new(nerust_core_traits::audio::AudioBackendRegistry::new());
    let session = SessionHandle::new_ephemeral(test_capabilities(), registry, audio_registry);
    assert!(!session.debug_write_byte(SpaceId(0), 0, 0xFF));
}

fn loaded_debug_session() -> SessionHandle {
    let mut session = test_session();
    let resolved = session
        .factory()
        .expect("no active system")
        .resolve_load_request(&test_view(&session), NoopSystemLoadOptions.into())
        .expect("load resolves");
    session
        .load_resolved(MediaObject::new(None, test_rom()), resolved)
        .expect("test ROM loads");
    // Real debugger opens pause first; the helper mirrors that instead
    // of relying on the load stamping a paused mirror.
    session
        .run_command(SessionCommand::Pause)
        .expect("debug sessions start paused");
    assert!(session.debug_paused());
    session
}

fn drain_input() -> crate::debug::DebugDrainInput<'static> {
    use crate::debug::DebugDrainInput;
    use nerust_core_traits::debugger::SpaceId;
    DebugDrainInput {
        space: Some(SpaceId(0)),
        mem_addr: 0,
        dis_addr: None,
        watch: None,
        freeze: None,
        reg_target: None,
        prev_dump_rows: &[],
        need_images: false,
    }
}

#[test]
fn debug_snapshot_covers_loaded_mock_core() {
    use nerust_core_traits::debugger::SpaceId;
    let session = loaded_debug_session();
    let snapshot = session.debug_snapshot(Some(SpaceId(0)), 0, None, true, Some(0x10));
    assert!(snapshot.available);
    assert_eq!(snapshot.reg_names, vec!["a", "b"]);
    assert_eq!(snapshot.regs.len(), 2);
    // Scroll window over the 256-byte mock space: 16 full rows,
    // then the range end stops the fetch.
    assert_eq!(snapshot.dump_rows.len(), 16);
    assert_eq!(snapshot.pc, Some(0x10));
    assert_eq!(snapshot.disasm_lines.len(), 32);
    assert!(snapshot.disasm_lines[0].is_pc);
    assert_eq!(snapshot.images.len(), 1);
    // Watch folds into the page read: fresh mock memory is zero.
    assert_eq!(snapshot.watch, Some(0));
    assert!(snapshot.panels.is_empty());
}

#[test]
fn debug_live_images_follow_running_without_snapshot() {
    use crate::debug::DebugAction;
    let session = loaded_debug_session();
    let input = drain_input();
    // Paused baseline carries dump rows.
    let paused = session.debug_drain(&[DebugAction::Refresh], &input);
    assert!(!paused.display.dump_rows.is_empty());
    assert!(session.debug_live_images(false).is_none());
    let paused_live = session.debug_live_images(true);
    let paused_gen = paused_live.as_ref().expect("paused fetch serves").0;
    assert_eq!(
        paused_live.map(|(_, images)| images.len()),
        Some(1),
        "paused baseline converts the decode"
    );
    // Resume: the full snapshot goes degenerate because the inspect
    // is paused-gated (running reads cannot pin a frame)...
    session.debug_drain(&[DebugAction::Resume], &input);
    let running = session.debug_drain(&[], &input);
    assert_eq!(running.display.status, StatusMessage::Running);
    assert!(
        running.display.dump_rows.is_empty(),
        "inspect refuses while running"
    );
    // ...but the live images path still flows, so the PPU follow
    // never needs (or blanks) the snapshot sections. Identical
    // content reports the served generation with no payload (the
    // content-hash skip): same pixels, same generation, zero UI work.
    let live = session.debug_live_images(true);
    let (running_gen, running_images) = live.expect("running fetch serves");
    assert_eq!(
        running_gen, paused_gen,
        "static tiles keep one generation while running"
    );
    assert!(
        running_images.is_empty(),
        "identical refetch skips the RGBA conversion"
    );
    session.debug_drain(&[DebugAction::Pause], &input);
}

#[test]
fn debug_drain_happy_path_reports_success() {
    use crate::debug::DebugAction;
    let session = loaded_debug_session();
    let input = drain_input();
    let paused = session.debug_drain(&[DebugAction::Pause], &input);
    assert_eq!(
        paused.display.status,
        session.debug_pause(),
        "pause round-trips status"
    );
    let stepped = session.debug_drain(&[DebugAction::StepFrame], &input);
    // Frame steps report the barrier frame counter (free-run may
    // precede the pause), so only the variant is pinned; wording is
    // the frontend's job (covered in the viewmodel render test).
    assert!(matches!(stepped.display.status, StatusMessage::Stepped(_)));
    // Memory write round-trips through the thread: stage, commit,
    // then read back the staged byte.
    let staged = session.debug_drain(
        &[DebugAction::SelectRow(0x20), DebugAction::EditStage(0xAB)],
        &input,
    );
    assert!(staged.display.pending.is_some());
    let committed = session.debug_drain(&[DebugAction::WriteConfirm], &input);
    assert_eq!(
        committed.display.status,
        StatusMessage::WroteMemory {
            addr: 0x20,
            value: 0xAB
        }
    );
    assert_eq!(
        session.debug_read_byte(nerust_core_traits::debugger::SpaceId(0), 0x20),
        Some(0xAB)
    );
    // Staging off the map commits loudly instead of writing.
    session.debug_prepare_write(nerust_core_traits::debugger::SpaceId(9), 0x10);
    session.debug_stage_write(0xAB);
    assert_eq!(
        session.debug_commit_write(),
        StatusMessage::WriteRefused("UnknownSpace(SpaceId(9))".to_string())
    );
    // Resume, toggle, instruction steps, and freeze re-apply ride
    // the same drain.
    let resumed = session.debug_drain(&[DebugAction::Resume], &input);
    assert_eq!(resumed.display.status, StatusMessage::Resume);
    // An empty batch on the running core reports running.
    let running = session.debug_drain(&[], &input);
    assert_eq!(running.display.status, StatusMessage::Running);
    let toggled = session.debug_drain(&[DebugAction::TogglePause], &input);
    assert_eq!(toggled.display.status, StatusMessage::Paused);
    let instr = session.debug_drain(&[DebugAction::StepInstr], &input);
    assert!(matches!(instr.display.status, StatusMessage::Stepped(_)));
    // Toggling while paused resumes back; pause again for the
    // deterministic reads below.
    let toggled_back = session.debug_drain(&[DebugAction::TogglePause], &input);
    assert_eq!(toggled_back.display.status, StatusMessage::Resume);
    session.debug_drain(&[DebugAction::Pause], &input);
    let mut frozen = drain_input();
    frozen.freeze = Some((0x30, 0xBB));
    session.debug_drain(&[DebugAction::Refresh], &frozen);
    assert_eq!(
        session.debug_read_byte(nerust_core_traits::debugger::SpaceId(0), 0x30),
        Some(0xBB)
    );
    // Register write round-trips the same way (wording is rendered
    // frontend-side; one spot-check pins the end-to-end text).
    let mut targeted = drain_input();
    targeted.reg_target = Some("a");
    let wrote = session.debug_drain(&[DebugAction::RegisterWrite(7)], &targeted);
    assert_eq!(
        wrote.display.status,
        StatusMessage::RegisterWrote {
            name: "a",
            value: 7
        }
    );
    assert_eq!(
        wrote.display.status.render(AppLanguage::English),
        "wrote 0x0007 to register a"
    );
    // Unknown names refuse loudly.
    assert_eq!(
        session.debug_write_register("bogus", 0),
        StatusMessage::RegisterRefused("Unsupported".to_string())
    );
    let after = session.debug_snapshot(
        Some(nerust_core_traits::debugger::SpaceId(0)),
        0,
        None,
        false,
        None,
    );
    assert_eq!(after.regs[0], ("a", 7));
}

#[test]
fn debug_drain_register_write_reports_loudly_without_core() {
    use crate::debug::{DebugAction, DebugDrainInput};
    use nerust_core_traits::debugger::SpaceId;
    let registry = Arc::new(SystemRegistry::new(vec![Arc::new(MockFactory)]));
    let audio_registry = Arc::new(nerust_core_traits::audio::AudioBackendRegistry::new());
    let session = SessionHandle::new_ephemeral(test_capabilities(), registry, audio_registry);
    let input = DebugDrainInput {
        space: Some(SpaceId(0)),
        mem_addr: 0,
        dis_addr: None,
        watch: None,
        freeze: None,
        reg_target: Some("r0"),
        prev_dump_rows: &[],
        need_images: false,
    };
    // Register writes ride the drain like memory commits: without a
    // core the refusal is loud, never silent.
    let output = session.debug_drain(&[DebugAction::RegisterWrite(0x42)], &input);
    assert_eq!(output.display.status, StatusMessage::NoCore);
    assert_eq!(
        output.display.status.render(AppLanguage::Japanese),
        "コアなし"
    );
    // No target, no write: the batch stays a pure re-read.
    let untargeted = DebugDrainInput {
        reg_target: None,
        ..input
    };
    let idle = session.debug_drain(&[DebugAction::RegisterWrite(0x42)], &untargeted);
    assert_eq!(idle.display.status, StatusMessage::Paused);
}

#[test]
fn drain_flight_poll_reaches_done_with_snapshot_sections() {
    use crate::debug::{DebugAction, DrainPoll};
    use std::time::{Duration, Instant};
    let session = loaded_debug_session();
    let input = drain_input();
    let mut flight = session.debug_drain_begin(&[DebugAction::Refresh], &input);
    // The UI-thread contract: one poll never blocks on the emu
    // thread, no matter how slow it is.
    let started = Instant::now();
    let first = flight.poll();
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "first poll must not park on the thread"
    );
    let output = match first {
        DrainPoll::Done(output) => output,
        DrainPoll::Pending => {
            let deadline = Instant::now() + Duration::from_secs(10);
            loop {
                match flight.poll() {
                    DrainPoll::Done(output) => break output,
                    DrainPoll::Pending => {
                        assert!(Instant::now() < deadline, "flight always terminates");
                        std::thread::sleep(Duration::from_millis(1));
                    }
                }
            }
        }
    };
    assert!(!output.display.dump_rows.is_empty());
    assert_eq!(output.display.status, StatusMessage::Paused);
}

#[test]
fn drain_flight_no_core_completes_on_first_poll() {
    use crate::debug::{DebugAction, DebugDrainInput, DrainPoll};
    let registry = Arc::new(SystemRegistry::new(vec![Arc::new(MockFactory)]));
    let audio_registry = Arc::new(nerust_core_traits::audio::AudioBackendRegistry::new());
    let session = SessionHandle::new_ephemeral(test_capabilities(), registry, audio_registry);
    let input = DebugDrainInput {
        space: None,
        mem_addr: 0,
        dis_addr: None,
        watch: None,
        freeze: None,
        reg_target: None,
        prev_dump_rows: &[],
        need_images: false,
    };
    // No thread crossings exist without a core: every phase resolves
    // locally, so one poll finishes (widget tests rely on this to
    // keep `act` synchronous in the coreless harness).
    let mut flight = session.debug_drain_begin(&[DebugAction::Pause, DebugAction::Refresh], &input);
    let DrainPoll::Done(output) = flight.poll() else {
        panic!("coreless flight must complete on the first poll");
    };
    assert_eq!(output.display.status, StatusMessage::NoCore);
    assert!(output.display.dump_rows.is_empty());
}

#[test]
fn live_images_flight_completes_and_resolves_locally() {
    use crate::debug::LiveImagesPoll;
    use std::time::{Duration, Instant};
    // Loaded core: spinning the flight serves the decode.
    let session = loaded_debug_session();
    let mut flight = session.debug_live_images_begin(true);
    let deadline = Instant::now() + Duration::from_secs(10);
    let live = loop {
        match flight.poll() {
            LiveImagesPoll::Done(live) => break live,
            LiveImagesPoll::Pending => {
                assert!(Instant::now() < deadline, "flight always terminates");
                std::thread::sleep(Duration::from_millis(1));
            }
        }
    };
    assert!(live.is_some_and(|(_, images)| images.len() == 1));
    // Not requested: the first poll resolves without crossings.
    let mut idle = session.debug_live_images_begin(false);
    assert!(matches!(idle.poll(), LiveImagesPoll::Done(None)));
    // No core: the first poll resolves too (tick polling never
    // parks, so coreless viewers stay synchronous).
    let registry = Arc::new(SystemRegistry::new(vec![Arc::new(MockFactory)]));
    let audio_registry = Arc::new(nerust_core_traits::audio::AudioBackendRegistry::new());
    let coreless = SessionHandle::new_ephemeral(test_capabilities(), registry, audio_registry);
    let mut flight = coreless.debug_live_images_begin(true);
    assert!(matches!(flight.poll(), LiveImagesPoll::Done(None)));
}

#[test]
fn live_images_flight_refetches_while_running() {
    use crate::debug::LiveImagesPoll;
    use std::time::{Duration, Instant};
    // Regression: consecutive begins on a running core must refetch,
    // not serve the first stored decode forever (live-freeze). The
    // mock tiles never change, so the refetch collapses to the served
    // generation with an empty payload — while the stale-cache bug
    // would serve one converted image at the same generation.
    let session = loaded_debug_session();
    session.debug_resume();
    assert!(!session.debug_paused());
    fn fetch(session: &SessionHandle) -> (u64, usize) {
        let mut flight = session.debug_live_images_begin(true);
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            match flight.poll() {
                LiveImagesPoll::Done(live) => {
                    break live.map(|(generation, images)| (generation, images.len()));
                }
                LiveImagesPoll::Pending => {
                    assert!(Instant::now() < deadline, "flight always terminates");
                    std::thread::sleep(Duration::from_millis(1));
                }
            }
        }
        .expect("running fetch serves")
    }
    let (first_gen, first_len) = fetch(&session);
    assert_eq!(first_len, 1, "first fetch converts the decode");
    let (second_gen, second_len) = fetch(&session);
    assert_eq!(
        second_gen, first_gen,
        "static tiles keep one generation while running"
    );
    assert_eq!(
        second_len, 0,
        "second begin refetches (identical collapse) instead of serving the stale cache"
    );
    session.debug_pause();
}

/// Empty queue: the pump skips drain/channel work, reports clean,
/// and leaves no rows or modal batches behind.
#[test]
fn pump_osd_empty_queue_reports_clean() {
    use std::time::Instant;

    use crate::session::osd::{ModalSource, OsdManager, PushInput, TransientPolicy};

    let mut session = test_session();
    let mut manager = OsdManager::new();
    let snapshot = session.settings_snapshot().clone();
    let input = || PushInput {
        policy: TransientPolicy::from(&snapshot.local.notifications),
        language: AppLanguage::English,
        now: Instant::now(),
    };
    // test_session activates a system but loads no ROM: unpresented.
    assert!(!session.loaded());
    let report = session.pump_osd(&mut manager, input());
    assert!(!report.changed);
    assert!(!report.modal_queued);
    assert!(manager.rows().is_empty());
    assert_eq!(manager.modal_pending(), 0);
    // Repeat: still clean, no accumulation.
    let report = session.pump_osd(&mut manager, input());
    assert!(!report.changed);
    assert!(!report.modal_queued);
}

/// No-ROM command failure (Error) still reaches the modal through
/// the pump: the desktop T7 behavior.
#[test]
fn pump_osd_routes_no_core_error_to_modal() {
    use std::time::Instant;

    use crate::session::osd::{ModalSource, OsdManager, PushInput, TransientPolicy};

    let registry = Arc::new(SystemRegistry::new(vec![Arc::new(MockFactory)]));
    let audio_registry = Arc::new(nerust_core_traits::audio::AudioBackendRegistry::new());
    let mut session = SessionHandle::new_ephemeral(test_capabilities(), registry, audio_registry);
    let mut manager = OsdManager::new();
    let snapshot = session.settings_snapshot().clone();
    // No core at all: Pause fails and pushes CommandFailed (Error).
    let outcome = session.run_command(SessionCommand::Pause);
    assert!(outcome.is_err());
    let report = session.pump_osd(
        &mut manager,
        PushInput {
            policy: TransientPolicy::from(&snapshot.local.notifications),
            language: AppLanguage::English,
            now: Instant::now(),
        },
    );
    assert!(report.modal_queued);
    let batch = manager.take_modal_batch().expect("error modal batch");
    assert!(batch.contains("Pause"));
    manager.modal_closed();
}

#[test]
fn delete_slot_without_core_reports_not_executed() {
    // No core ran anything: the outcome must not claim a deletion.
    let registry = Arc::new(SystemRegistry::new(vec![Arc::new(MockFactory)]));
    let audio_registry = Arc::new(nerust_core_traits::audio::AudioBackendRegistry::new());
    let mut session = SessionHandle::new_ephemeral(test_capabilities(), registry, audio_registry);
    let outcome = session
        .run_command(SessionCommand::DeleteSlot(1))
        .expect("delete reports an outcome");
    assert!(!outcome.executed);
    assert_eq!(outcome.slot_failure, Some(SlotOpFailure::Empty));
}

#[test]
fn report_external_error_truncates_and_labels_source() {
    use crate::session::notification::{FailureOrigin, MAX_EXTERNAL_DETAIL, NotificationKind};

    let mut session = test_session();
    let long = "x".repeat(MAX_EXTERNAL_DETAIL + 100);
    session.report_external_error(FailureOrigin::Bridge, long);
    let notes = session.drain_notifications();
    assert_eq!(notes.len(), 1);
    match &notes[0].kind {
        NotificationKind::CommandFailure {
            origin: source,
            detail,
            ..
        } => {
            assert_eq!(*source, FailureOrigin::Bridge);
            assert_eq!(detail.len(), MAX_EXTERNAL_DETAIL);
        }
        other => panic!("unexpected kind: {other:?}"),
    }
    assert_eq!(
        notes[0].level,
        crate::session::notification::NotificationLevel::Error
    );
}

/// Info without a composited frame is dropped by the pump: no generation
/// bump, no rows, no modal. Warnings still reach the modal.
#[test]
fn pump_osd_drops_info_without_frame() {
    use std::time::Instant;

    use crate::session::{
        notification::NotificationKind,
        osd::{ModalSource, OsdManager, PushInput, TransientPolicy},
    };

    let registry = Arc::new(SystemRegistry::new(vec![Arc::new(MockFactory)]));
    let audio_registry = Arc::new(nerust_core_traits::audio::AudioBackendRegistry::new());
    let mut session = SessionHandle::new_ephemeral(test_capabilities(), registry, audio_registry);
    assert!(!session.loaded());
    let mut manager = OsdManager::new();
    let snapshot = session.settings_snapshot().clone();
    let input = || PushInput {
        policy: TransientPolicy::from(&snapshot.local.notifications),
        language: AppLanguage::English,
        now: Instant::now(),
    };
    session.notifications.push(NotificationKind::Paused);
    let report = session.pump_osd(&mut manager, input());
    assert!(!report.changed);
    assert!(!report.modal_queued);
    assert!(manager.rows().is_empty());
    assert_eq!(manager.modal_pending(), 0);
    session
        .notifications
        .push(NotificationKind::PersistenceUnavailable);
    let report = session.pump_osd(&mut manager, input());
    assert!(report.modal_queued);
    assert_eq!(manager.modal_pending(), 1);
}
