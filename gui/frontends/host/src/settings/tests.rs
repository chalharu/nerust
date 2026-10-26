use nerust_settings_core::model::{
    app_state::DesktopAppState, input::ShortcutAction, local::HostBackendLocalSettings,
    shared::DesktopSharedSettings,
};

use super::{SettingsEvent as E, *};

fn empty_snapshot() -> SettingsSnapshot {
    SettingsSnapshot {
        shared: DesktopSharedSettings::default(),
        local: HostBackendLocalSettings::default(),
        app_state: DesktopAppState::default(),
    }
}

fn empty_state() -> SettingsController {
    SettingsController::new(
        &empty_snapshot(),
        Arc::new(SystemRegistry::new(Vec::new())),
        Arc::new(AudioBackendRegistry::new()),
    )
}

fn nes_state() -> SettingsController {
    SettingsController::new(
        &empty_snapshot(),
        Arc::new(SystemRegistry::new(vec![
            nerust_nes_factory::NesFactory::boxed().into(),
        ])),
        Arc::new(AudioBackendRegistry::new()),
    )
}

fn choice<T: Clone + Eq>(value: T) -> ChoiceView<T> {
    ChoiceView {
        value,
        label: String::new(),
    }
}

#[test]
fn view_model_provides_projection_values() {
    let state = empty_state();
    let general = state.vm.general.view.get();
    assert_eq!(general.language, AppLanguage::SystemDefault);
    assert_eq!(general.language_choices.len(), 3);
    let video = state.vm.video.view.get();
    assert!(!video.fullscreen_default);
    assert_eq!(video.scaling_choices.len(), 6);
    let audio = state.vm.audio.view.get();
    assert!(!audio.muted);
    assert_eq!(audio.volume_percent, 100);
    assert_eq!(
        audio
            .sample_rate_choices
            .iter()
            .map(|c| c.value)
            .collect::<Vec<_>>(),
        [44_100, 48_000]
    );
}

#[test]
fn update_applies_general_video_and_audio_messages() {
    let mut state = empty_state();
    let revision = state.vm.revision.get();
    for event in [
        E::SelectPage(SettingsPage::Audio),
        E::SelectSystemTab(2),
        E::SelectInputTab(3),
        E::SetLanguage(choice(AppLanguage::Japanese)),
        E::SetStoragePolicy(choice(StoragePolicy::CustomDirectory)),
        E::SetStorageDirectory("/tmp/states".into()),
        E::ToggleFullscreenDefault(true),
        E::SetScaling(choice(ScalingMode::X3)),
        E::ToggleVsync(false),
        E::ToggleMute(true),
        E::SetVolume(42),
        E::SetSampleRate(choice(44_100)),
        E::SetLatency(75),
    ] {
        assert!(state.update(event).is_none());
    }
    assert_eq!(state.page, SettingsPage::Audio);
    assert_eq!(state.system_tab_index, Some(2));
    assert_eq!(state.input_tab, Some(InputTab::System(3)));
    assert!(state.vm.revision.get() > revision);
    let general = state.vm.general.view.get();
    assert_eq!(general.language, AppLanguage::Japanese);
    assert_eq!(general.storage_policy, StoragePolicy::CustomDirectory);
    assert_eq!(
        state
            .vm
            .snapshot()
            .shared
            .persistence
            .storage_directory
            .as_deref(),
        Some(std::path::Path::new("/tmp/states"))
    );
    let video = state.vm.video.view.get();
    assert!(video.fullscreen_default);
    assert_eq!(video.scaling, ScalingMode::X3);
    assert!(!video.vsync);
    let audio = state.vm.audio.view.get();
    assert!(audio.muted);
    assert_eq!(audio.volume_percent, 42);
    assert_eq!(audio.sample_rate, 44_100);
    assert_eq!(audio.latency_ms, 75);
}

#[test]
fn submit_and_cancel_publish_close_state() {
    let mut submitted = empty_state();
    submitted.update(E::SetVolume(42));
    submitted.update(E::Submit);
    assert!(submitted.should_close.load(Ordering::Acquire));
    assert!(lock_recovered(&submitted.pending_apply).is_some());
    let mailbox = submitted.mailbox.clone();
    assert_eq!(
        mailbox
            .take_pending_apply()
            .unwrap()
            .local
            .audio
            .master_volume_percent,
        42
    );
    assert!(mailbox.take_pending_apply().is_none());

    let mut cancelled = empty_state();
    cancelled.update(E::SetVolume(42));
    cancelled.update(E::Cancel);
    assert!(cancelled.should_close.load(Ordering::Acquire));
    assert!(lock_recovered(&cancelled.pending_apply).is_none());
}

#[test]
fn validation_errors_empty_with_default_settings() {
    let mut state = empty_state();
    assert!(state.validation_errors().is_empty());
    assert!(state.storage_error().is_none());
    state.update(E::Submit);
    assert!(lock_recovered(&state.pending_apply).is_some());
}

#[test]
fn failed_validation_does_not_close_or_publish() {
    let mut state = empty_state();
    state.update(E::SetStoragePolicy(choice(StoragePolicy::CustomDirectory)));
    assert!(state.storage_error().is_some());
    assert!(!state.validation_errors().is_empty());
    state.update(E::Submit);
    assert!(!state.should_close.load(Ordering::Acquire));
    assert!(lock_recovered(&state.pending_apply).is_none());
}

#[test]
fn browse_cancellation_preserves_text_and_draft_and_selection_updates_both() {
    let mut state = empty_state();
    state.update(E::SetStorageDirectory("/original".into()));
    assert_eq!(
        state.update(E::BrowseStorageDirectory),
        Some(SettingsEffect::BrowseStorageDirectory {
            title: ui_text(state.language(), UiText::SaveStorageDirectory).into(),
        })
    );
    let revision = state.vm.revision.get();
    state.view_invalidated.store(false, Ordering::Release);
    state.update(E::StorageDirectorySelected(None));
    assert_eq!(state.storage_directory_input, "/original");
    assert_eq!(state.vm.general.view.get().storage_directory, "/original");
    assert_eq!(state.vm.revision.get(), revision);
    assert!(!state.view_invalidated.load(Ordering::Acquire));
    state.update(E::StorageDirectorySelected(Some(PathBuf::from(
        "/selected",
    ))));
    assert_eq!(state.storage_directory_input, "/selected");
    assert_eq!(state.vm.general.view.get().storage_directory, "/selected");
    assert!(state.view_invalidated.load(Ordering::Acquire));
}

#[test]
fn revision_callback_shares_external_view_invalidated_flag() {
    let external = Arc::new(AtomicBool::new(false));
    let state = SettingsController::new_with_shared(
        &empty_snapshot(),
        Arc::new(SystemRegistry::new(Vec::new())),
        Arc::new(AudioBackendRegistry::new()),
        Arc::new(AtomicBool::new(false)),
        Arc::new(Mutex::new(None)),
        Arc::clone(&external),
    );
    let revision = state.vm.revision.get();
    // Direct mutations still invalidate: the controller owns the subscription.
    state
        .vm
        .general
        .set_language(AppLanguage::Japanese)
        .unwrap();
    assert!(state.vm.revision.get() > revision);
    assert!(external.load(Ordering::Acquire));
}

#[test]
fn local_navigation_invalidates_without_vm_revision() {
    let mut state = empty_state();
    let revision = state.vm.revision.get();
    for event in [
        E::SelectPage(SettingsPage::Audio),
        E::SelectInputSection(InputPageSection::Attachment(2)),
        E::SelectSystemTab(2),
        E::SelectInputTab(3),
    ] {
        state.view_invalidated.store(false, Ordering::Release);
        state.update(event);
        assert!(state.view_invalidated.load(Ordering::Acquire));
        assert_eq!(state.vm.revision.get(), revision);
    }
}

#[test]
fn error_creation_and_clearing_invalidate_without_revision() {
    let mut state = nes_state();
    let revision = state.vm.revision.get();
    state.view_invalidated.store(false, Ordering::Release);
    state.update(E::SetControllerSlot {
        slot: AttachmentId::new("nonexistent.slot"),
        controller_id: None,
    });
    assert!(state.error_message.is_some());
    assert!(state.view_invalidated.load(Ordering::Acquire));
    assert_eq!(state.vm.revision.get(), revision);
    state.view_invalidated.store(false, Ordering::Release);
    // Key without an active capture is a VM no-op, but clears the error banner.
    state.update(E::CaptureKey(Key::Space));
    assert!(state.error_message.is_none());
    assert!(state.view_invalidated.load(Ordering::Acquire));
    assert_eq!(state.vm.revision.get(), revision);
}

#[test]
fn capture_messages_update_capture_state() {
    let mut snapshot = empty_snapshot();
    snapshot.shared.input.shortcuts.keyboard.push(
        nerust_settings_core::model::input::ShortcutBinding {
            action: ShortcutAction::TogglePause,
            key: None,
        },
    );
    let mut state = SettingsController::new(
        &snapshot,
        Arc::new(SystemRegistry::new(Vec::new())),
        Arc::new(AudioBackendRegistry::new()),
    );
    let target = CaptureTarget::Shortcut(ShortcutAction::TogglePause);
    state.update(E::StartCapture(target.clone()));
    assert_eq!(state.vm.capture.view.get().target, Some(target.clone()));
    assert!(state.view_invalidated.swap(false, Ordering::AcqRel));
    state.update(E::CaptureKey(Key::Space));
    assert!(state.vm.capture.view.get().target.is_none());
    assert!(state.view_invalidated.swap(false, Ordering::AcqRel));
    assert_eq!(
        nerust_settings_core::editor::current_binding_label(&state.vm.snapshot(), &target),
        Some(Key::Space.label())
    );
    state.update(E::StartCapture(target.clone()));
    state.update(E::ClearCapture(target.clone()));
    assert!(state.vm.capture.view.get().target.is_none());
    assert_eq!(
        nerust_settings_core::editor::current_binding_label(&state.vm.snapshot(), &target),
        None
    );
}

#[test]
fn select_input_tab_maps_trailing_index_to_shortcuts_and_resets_section() {
    let mut state = empty_state();
    assert_eq!(state.system_tab_index, None);
    assert_eq!(state.input_tab, None);
    state.update(E::SelectInputSection(InputPageSection::Attachment(2)));
    assert_eq!(state.input_section, InputPageSection::Attachment(2));
    state.update(E::SelectInputTab(0));
    assert_eq!(state.input_tab, Some(InputTab::Shortcuts));
    assert_eq!(state.input_section, InputPageSection::Attachment(0));
    state.update(E::SelectInputTab(2));
    assert_eq!(state.input_tab, Some(InputTab::System(2)));
}

#[test]
fn empty_and_out_of_bounds_selectors_remain_safe() {
    let mut state = empty_state();
    for event in [
        E::SetSystemChoice(
            "unknown".into(),
            choice(SystemSettingsChoiceId("none".into())),
        ),
        E::SetControllerSlot {
            slot: AttachmentId::new("nonexistent.slot"),
            controller_id: None,
        },
        E::SelectInputTab(9),
        E::SelectSystemTab(9),
        E::SetSystemChoice(
            "unknown".into(),
            choice(SystemSettingsChoiceId("none".into())),
        ),
        E::SetControllerSlot {
            slot: AttachmentId::new("nonexistent.slot"),
            controller_id: None,
        },
    ] {
        state.update(event);
        assert!(state.error_message.is_none());
    }
}

#[test]
fn controller_and_system_selectors_reach_the_selected_vm() {
    let mut state = nes_state();
    assert_eq!(state.system_tab_index, Some(0));
    assert_eq!(state.input_tab, Some(InputTab::System(0)));
    let slot = state.vm.inputs()[0].view.get().slots[0].clone();
    let profile = slot
        .choices
        .iter()
        .find(|c| c.value.is_some())
        .unwrap()
        .value
        .clone();
    state.update(E::SetControllerSlot {
        slot: slot.slot_id,
        controller_id: None,
    });
    assert!(
        state.vm.inputs()[0].view.get().slots[0]
            .selected_profile_id
            .is_none()
    );
    state.update(E::SetControllerSlot {
        slot: slot.slot_id,
        controller_id: profile.clone(),
    });
    assert_eq!(
        state.vm.inputs()[0].view.get().slots[0].selected_profile_id,
        profile
    );
    let field = state.vm.systems()[0].view.get().fields[0].clone();
    let selected = field
        .choices
        .iter()
        .find(|c| c.value != field.selected)
        .unwrap()
        .clone();
    state.update(E::SetSystemChoice(field.id.0.to_string(), selected.clone()));
    assert_eq!(
        state.vm.systems()[0].view.get().fields[0].selected,
        selected.value
    );
    assert!(state.error_message.is_none());
}

fn key(entry: &(&str, u32)) -> String {
    entry.0.to_string()
}

#[test]
fn first_duplicate_reports_first_repeat_in_order() {
    assert_eq!(
        first_duplicate_key(vec![("a", 1), ("b", 2), ("a", 3), ("b", 4)], key),
        Some("a".into())
    );
}

#[test]
fn first_duplicate_returns_none_without_repeats() {
    assert_eq!(first_duplicate_key(vec![("a", 1), ("b", 2)], key), None);
    assert_eq!(first_duplicate_key(Vec::<(&str, u32)>::new(), key), None);
}

#[test]
fn dedupe_keeps_first_and_reports_drops() {
    let mut dropped = Vec::new();
    let kept = deduped_by_key(vec![("a", 1), ("b", 2), ("a", 3)], key, |item| {
        dropped.push(item.1)
    });
    assert_eq!(kept, vec![("a", 1), ("b", 2)]);
    assert_eq!(dropped, vec![3]);
}

#[test]
fn catalog_dedupes_system_ids_before_registry_construction() {
    let first: Arc<dyn CoreFactory> = nerust_nes_factory::NesFactory::boxed().into();
    let factories = vec![
        first.clone(),
        nerust_nes_factory::NesFactory::boxed().into(),
    ];
    // SystemRegistry itself rejects duplicates. Exercise the same key/dedupe
    // policy used by the controller before constructing a valid registry.
    assert_eq!(
        first_duplicate_key(&factories, |factory| system_id_key(factory)),
        Some(first.system_id().to_string())
    );
    let deduped = deduped_by_key(factories, system_id_key, |_| {});
    assert_eq!(deduped.len(), 1);
    assert!(Arc::ptr_eq(&deduped[0], &first));
    let registry = Arc::new(SystemRegistry::new(deduped));
    assert!(find_duplicate_system_id(&registry).is_none());
    assert!(Arc::ptr_eq(&deduped_factories(&registry)[0], &first));
    let state = SettingsController::new(
        &empty_snapshot(),
        registry,
        Arc::new(AudioBackendRegistry::new()),
    );
    assert_eq!(state.vm.systems().len(), 1);
    assert_eq!(state.vm.inputs().len(), 1);
}

#[test]
fn notification_catalogs_preserve_order_and_localize() {
    let english = NotificationChoices::new(AppLanguage::English);
    let japanese = NotificationChoices::new(AppLanguage::Japanese);
    assert_eq!(english.colors.len(), 5);
    assert_eq!(english.positions.len(), 4);
    assert_eq!(english.areas.len(), 2);
    assert_eq!(english.colors[0].value, NotificationColor::White);
    assert_eq!(english.colors[4].value, NotificationColor::Cyan);
    assert_eq!(english.positions[0].value, NotificationPosition::TopLeft);
    assert_eq!(
        english.positions[3].value,
        NotificationPosition::BottomRight
    );
    assert_eq!(english.areas[0].value, NotificationAreaMode::Overlay);
    assert_eq!(english.areas[1].value, NotificationAreaMode::Strip);
    assert_ne!(english.colors[0].label, japanese.colors[0].label);
}

#[test]
fn local_text_change_invalidates_even_when_vm_already_has_the_value() {
    let mut state = empty_state();
    state
        .vm
        .general
        .set_storage_directory(Some("/selected".into()))
        .unwrap();
    let revision = state.vm.revision.get();
    state.view_invalidated.store(false, Ordering::Release);
    state.update(E::SetStorageDirectory("/selected".into()));
    assert_eq!(state.vm.revision.get(), revision);
    assert_eq!(state.storage_directory_input, "/selected");
    assert!(state.view_invalidated.load(Ordering::Acquire));
    state.update(E::SetStorageDirectory(String::new()));
    assert!(
        state
            .vm
            .snapshot()
            .shared
            .persistence
            .storage_directory
            .is_none()
    );
}

#[test]
fn notification_editor_messages_reach_all_projections() {
    let mut state = empty_state();
    for event in [
        E::ToggleInfoEnabled(false),
        E::SetInfoDuration(2345),
        E::SetInfoColor(choice(NotificationColor::Green)),
        E::ToggleWarningEnabled(false),
        E::SetWarningDuration(4567),
        E::SetWarningColor(choice(NotificationColor::Cyan)),
        E::ToggleErrorEnabled(false),
        E::ToggleErrorModal(false),
        E::SetErrorColor(choice(NotificationColor::Yellow)),
        E::ToggleStatusEnabled(false),
        E::SetPosition(choice(NotificationPosition::BottomLeft)),
        E::SetStatusPosition(choice(NotificationPosition::TopRight)),
        E::SetAreaMode(choice(NotificationAreaMode::Strip)),
        E::SetStatusAreaMode(choice(NotificationAreaMode::Strip)),
    ] {
        state.update(event);
        assert!(state.error_message.is_none());
    }
    let view = state.vm.notifications.view.get();
    assert!(!view.info_enabled);
    assert_eq!(view.info_duration_ms, 2345);
    assert_eq!(view.info_color, NotificationColor::Green);
    assert!(!view.warning_enabled);
    assert_eq!(view.warning_duration_ms, 4567);
    assert_eq!(view.warning_color, NotificationColor::Cyan);
    assert!(!view.error_enabled);
    assert!(!view.error_modal);
    assert_eq!(view.error_color, NotificationColor::Yellow);
    assert!(!view.status_enabled);
    assert_eq!(view.position, NotificationPosition::BottomLeft);
    assert_eq!(view.status_position, NotificationPosition::TopRight);
    assert_eq!(view.area_mode, NotificationAreaMode::Strip);
    assert_eq!(view.status_area_mode, NotificationAreaMode::Strip);
}
