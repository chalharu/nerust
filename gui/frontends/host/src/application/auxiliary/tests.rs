use super::*;
use crate::application::tests::Fixture;
use crate::{widget_session::WidgetSession, window::WidgetContents};
use nerust_gui_shell::registry::SystemRegistry;
use wakui::control::{
    Control, ControlDisplay, ControlEvent, ControlInput, ControlKind, ControlVisitor,
};
use wakui::pause::PauseOwner;

fn prepare(
    host: &mut ApplicationHost,
    windows: &mut AuxWindows<u64, ()>,
    kind: AuxKind,
) -> (AuxId, WidgetSession) {
    let Some(OpenRequest::Create(id)) = host.reserve_window(windows, kind).unwrap() else {
        panic!()
    };
    let session = host.prepare_widget_window(windows, id).unwrap().unwrap();
    assert!(matches!(
        windows.window(id).unwrap().contents.contents,
        WidgetContents::Controls(_)
    ));
    assert!(windows.complete_open(id, Some(1), ()).is_ok());
    (id, session)
}

fn controls(session: &WidgetSession) -> Vec<Control> {
    #[derive(Default)]
    struct Controls(Vec<Control>);
    impl ControlVisitor for Controls {
        fn control(&mut self, control: &Control) {
            self.0.push(control.clone());
        }
    }
    let mut controls = Controls::default();
    session.screen().visit(&mut controls);
    controls.0
}

fn input(session: &mut WidgetSession, predicate: impl Fn(&Control) -> bool, value: ControlInput) {
    let id = controls(session)
        .iter()
        .find(|c| predicate(c))
        .expect("generic control")
        .id;
    assert!(session.dispatch(ControlEvent::new(id, value)).is_empty());
}

fn volume(session: &mut WidgetSession, value: u32) {
    input(
        session,
        |c| matches!(c.kind, ControlKind::Number { max: 100, .. }),
        ControlInput::Number(value),
    );
}

fn submit(session: &mut WidgetSession) {
    input(
        session,
        |c| c.display == ControlDisplay::Primary,
        ControlInput::Activate,
    );
}

#[test]
fn opaque_submission_returns_real_effects_and_clones_cannot_consume_future_drafts() {
    use nerust_settings_core::i18n::{UiText, text};
    let mut fixture = Fixture::new();
    fixture.load();
    let host = &mut fixture.core;
    let mut windows = AuxWindows::new();
    let (id, mut session) = prepare(host, &mut windows, AuxKind::Settings);
    host.set_pause_owner(PauseOwner::Modal, true);
    let language = host.settings_snapshot().shared.general.language;
    input(
        &mut session,
        |c| c.label == text(language, UiText::FullscreenDefault),
        ControlInput::Bool(true),
    );
    // Choice ordering is owned by the Host projection, not the native adapter.
    input(
        &mut session,
        |c| c.label == text(language, UiText::Scaling),
        ControlInput::Select(2),
    );
    volume(&mut session, 38);
    submit(&mut session);
    let mailbox = windows.mailbox(id).unwrap();
    assert!(mailbox.close_requested());
    let submission = mailbox.take_submission().unwrap();
    let duplicate = submission.clone();
    let effects = host
        .apply_window_submission(&windows, id, submission)
        .unwrap();
    assert_eq!(effects.fullscreen, Some(true));
    assert!(effects.redraw && effects.refresh_chrome);
    assert!(effects.rebuild_renderer);
    let size = host.session().window_size();
    let scale = nerust_gui_shell::settings::scaling_factor(
        host.settings_snapshot().local.video.window.scaling,
    )
    .unwrap();
    assert_eq!(
        effects.resize,
        Some((
            (size.width * scale as f32).round() as u32,
            (size.height * scale as f32).round() as u32
        ))
    );
    assert_eq!(
        host.settings_snapshot().local.audio.master_volume_percent,
        38
    );
    assert!(host.owns_pause(PauseOwner::Settings));
    volume(&mut session, 41);
    submit(&mut session);
    assert_eq!(
        host.apply_window_submission(&windows, id, duplicate)
            .unwrap(),
        WindowEffects::default()
    );
    assert!(
        mailbox.has_pending(),
        "old clone must not steal next submission"
    );
    let (handle, result) = host
        .close_widget_window(&mut windows, id, mailbox.take_submission())
        .unwrap();
    assert_eq!(handle, Some(()));
    assert!(result.is_ok());
    assert_eq!(
        host.settings_snapshot().local.audio.master_volume_percent,
        41
    );
    assert!(!host.owns_pause(PauseOwner::Settings));
    assert!(host.owns_pause(PauseOwner::Modal));
    assert!(host.tick().paused);
}

#[test]
fn save_failure_rolls_back_but_preserves_window_draft_and_pause_for_retry() {
    let mut fixture = Fixture::new();
    fixture.load();
    let path = nerust_gui_runtime::settings::SettingsPaths::from_root(&fixture.root).settings_file;
    if path.is_file() {
        std::fs::remove_file(&path).unwrap();
    }
    std::fs::create_dir_all(&path).unwrap();
    let host = &mut fixture.core;
    let original = host.settings_snapshot().clone();
    let mut windows = AuxWindows::new();
    let (id, mut session) = prepare(host, &mut windows, AuxKind::Settings);
    volume(&mut session, 26);
    submit(&mut session);
    let mailbox = session.mailbox();
    let pending = mailbox.take_submission().unwrap();
    assert!(host.apply_window_submission(&windows, id, pending).is_err());
    assert_eq!(host.settings_snapshot(), &original);
    assert!(windows.contains(AuxKind::Settings));
    assert!(host.owns_pause(PauseOwner::Settings));
    assert!(host.tick().paused);
    assert!(!mailbox.close_requested());
    assert!(mailbox.take_invalidated());
    assert!(session.screen().message.is_some());
    assert!(controls(&session).iter().any(|c| matches!(
        c.kind,
        ControlKind::Number {
            value: 26,
            max: 100,
            ..
        }
    )));
    std::fs::remove_dir(&path).unwrap();
    submit(&mut session);
    let pending = mailbox.take_submission().unwrap();
    assert!(
        host.apply_window_submission(&windows, id, pending)
            .unwrap()
            .redraw
    );
    assert_eq!(
        host.settings_snapshot().local.audio.master_volume_percent,
        26
    );
    assert!(session.screen().message.is_none());
    assert!(
        host.close_widget_window(&mut windows, id, None)
            .unwrap()
            .1
            .is_ok()
    );
    assert!(!host.owns_pause(PauseOwner::Settings));
}

#[test]
fn generic_history_filter_clear_close_mutate_real_session_and_publish_rows() {
    let mut fixture = Fixture::new();
    let host = &mut fixture.core;
    host.report_host_error("generic history row".into());
    let mut windows = AuxWindows::new();
    let (id, mut session) = prepare(host, &mut windows, AuxKind::History);
    assert!(!host.pause_held());
    input(
        &mut session,
        |c| matches!(&c.kind, ControlKind::Note { text } if text.contains("generic history row")),
        ControlInput::Activate,
    );
    input(
        &mut session,
        |c| matches!(c.kind, ControlKind::Choice { .. }),
        ControlInput::Select(3),
    );
    let language = host.settings_snapshot().shared.general.language;
    let clear_label = nerust_settings_core::i18n::text(
        language,
        nerust_settings_core::i18n::UiText::HistoryClear,
    );
    input(
        &mut session,
        |c| c.label == clear_label,
        ControlInput::Activate,
    );
    let mailbox = windows.mailbox(id).unwrap();
    let effects = host
        .apply_window_submission(&windows, id, mailbox.take_submission().unwrap())
        .unwrap();
    assert!(effects.redraw);
    assert_eq!(host.session().notification_history().count(), 0);
    host.refresh_widget_session(&mut session);
    assert!(mailbox.take_invalidated());
    assert!(matches!(
        session.window().contents.contents,
        WidgetContents::Controls(_)
    ));
    let close_label =
        nerust_settings_core::i18n::text(language, nerust_settings_core::i18n::UiText::Close);
    input(
        &mut session,
        |c| c.label == close_label,
        ControlInput::Activate,
    );
    assert!(mailbox.close_requested());
    assert!(
        host.close_widget_window(&mut windows, id, None)
            .unwrap()
            .1
            .is_ok()
    );
    assert!(!windows.contains(AuxKind::History));
    assert!(host.history_key().is_none());
}

#[test]
fn generation_and_mailbox_identity_guard_native_close_and_stale_submission() {
    let mut fixture = Fixture::new();
    let host = &mut fixture.core;
    let mut windows = AuxWindows::new();
    let (old, mut old_session) = prepare(host, &mut windows, AuxKind::Settings);
    volume(&mut old_session, 12);
    submit(&mut old_session);
    let stale = old_session.mailbox().take_submission().unwrap();
    assert!(host.close_widget_window(&mut windows, old, None).is_some());
    let (current, _) = prepare(host, &mut windows, AuxKind::Settings);
    assert_ne!(old, current);
    assert!(
        host.close_widget_window(&mut windows, old, Some(stale.clone()))
            .is_none()
    );
    assert!(
        host.close_widget_window(&mut windows, current, Some(stale.clone()))
            .is_none()
    );
    assert_eq!(
        host.apply_window_submission(&windows, current, stale.clone())
            .unwrap(),
        WindowEffects::default()
    );
    assert_eq!(
        host.apply_window_submission(&windows, old, stale).unwrap(),
        WindowEffects::default()
    );
    assert!(host.owns_pause(PauseOwner::Settings));
    assert_ne!(
        host.settings_snapshot().local.audio.master_volume_percent,
        12
    );
    assert!(
        host.close_widget_window(&mut windows, current, None)
            .is_some()
    );
    assert!(!host.owns_pause(PauseOwner::Settings));
}

#[test]
fn close_before_apply_failure_releases_only_settings_and_retires_generation() {
    let mut fixture = Fixture::new();
    fixture.load();
    let path = nerust_gui_runtime::settings::SettingsPaths::from_root(&fixture.root).settings_file;
    if path.is_file() {
        std::fs::remove_file(&path).unwrap();
    }
    std::fs::create_dir_all(&path).unwrap();
    let host = &mut fixture.core;
    let original = host.settings_snapshot().clone();
    let mut windows = AuxWindows::new();
    let (id, mut session) = prepare(host, &mut windows, AuxKind::Settings);
    host.set_pause_owner(PauseOwner::Modal, true);
    host.note_main_departure();
    host.tick_focus_sync();
    assert!(host.owns_pause(PauseOwner::Focus));
    volume(&mut session, 13);
    submit(&mut session);
    let pending = session.mailbox().take_submission().unwrap();
    let duplicate = pending.clone();
    let (handle, result) = host
        .close_widget_window(&mut windows, id, Some(pending))
        .unwrap();
    assert_eq!(handle, Some(()));
    assert!(result.is_err());
    assert!(!windows.contains(AuxKind::Settings));
    assert!(!host.owns_pause(PauseOwner::Settings));
    assert!(host.owns_pause(PauseOwner::Modal));
    assert!(host.owns_pause(PauseOwner::Focus));
    assert!(host.tick().paused);
    assert_eq!(host.settings_snapshot(), &original);
    assert!(
        host.close_widget_window(&mut windows, id, Some(duplicate))
            .is_none()
    );
    std::fs::remove_dir(&path).unwrap();
}

fn reserve(host: &mut ApplicationHost, windows: &mut AuxWindows<u64, ()>) -> AuxId {
    let Some(OpenRequest::Create(id)) = host.reserve_window(windows, AuxKind::Settings).unwrap()
    else {
        panic!("expected fresh settings reservation");
    };
    id
}

#[test]
fn shared_success_applies_before_release_and_preserves_modal_owner() {
    let mut fixture = Fixture::new();
    fixture.load();
    let host = &mut fixture.core;
    let mut windows = AuxWindows::new();
    let id = reserve(host, &mut windows);
    assert!(host.tick().paused);
    let mut session = host
        .prepare_widget_window(&mut windows, id)
        .unwrap()
        .unwrap();
    assert_eq!(windows.window(id).unwrap().title, session.window().title);
    assert!(windows.complete_open(id, Some(1), ()).is_ok());
    host.set_pause_owner(PauseOwner::Modal, true);
    submit(&mut session);
    let (handle, result) = host
        .close_widget_window(&mut windows, id, session.mailbox().take_submission())
        .unwrap();
    assert_eq!(handle, Some(()));
    assert!(result.unwrap().refresh_chrome);
    assert!(!host.owns_pause(PauseOwner::Settings));
    assert!(host.owns_pause(PauseOwner::Modal));
    assert!(host.tick().paused);
}

#[test]
fn pending_present_failure_and_stale_callbacks_do_not_touch_new_owner() {
    let mut fixture = Fixture::new();
    let host = &mut fixture.core;
    let mut windows = AuxWindows::new();
    let old = reserve(host, &mut windows);
    assert_eq!(
        host.reserve_window(&mut windows, AuxKind::Settings)
            .unwrap(),
        Some(OpenRequest::Pending(old))
    );
    assert!(host.cancel_window(&mut windows, old));
    assert!(!host.pause_held());
    let current = reserve(host, &mut windows);
    assert_ne!(old, current);
    assert!(!host.cancel_window(&mut windows, old));
    assert!(host.close_widget_window(&mut windows, old, None).is_none());
    assert!(host.owns_pause(PauseOwner::Settings));
    assert!(windows.complete_open(current, Some(2), ()).is_ok());
    assert_eq!(
        host.reserve_window(&mut windows, AuxKind::Settings)
            .unwrap(),
        Some(OpenRequest::Present(current))
    );
    assert!(!host.cancel_window(&mut windows, current));
    assert!(host.owns_pause(PauseOwner::Settings));
    assert!(
        host.close_widget_window(&mut windows, current, None)
            .unwrap()
            .1
            .unwrap()
            .refresh_chrome
    );
    assert!(!host.pause_held());
}

#[test]
fn duplicate_factory_registration_is_rejected_and_reservation_can_roll_back() {
    let mut fixture = Fixture::new();
    let host = &mut fixture.core;
    let mut windows = AuxWindows::new();
    let id = reserve(host, &mut windows);
    // A real registry cannot contain duplicates. Do not fabricate an invalid
    // registry with unsafe layout tricks merely to exercise defensive checking.
    let registry = std::panic::catch_unwind(|| {
        SystemRegistry::new(vec![
            nerust_nes_factory::NesFactory::boxed().into(),
            nerust_nes_factory::NesFactory::boxed().into(),
        ])
    });
    assert!(registry.is_err());
    assert!(host.cancel_window(&mut windows, id));
    assert!(!windows.contains(AuxKind::Settings));
    assert!(!host.owns_pause(PauseOwner::Settings));
}

#[test]
fn invalid_draft_cannot_submit_and_cancel_releases_only_settings_pause() {
    use nerust_settings_core::i18n::{UiText, text};
    let mut fixture = Fixture::new();
    let host = &mut fixture.core;
    let mut windows = AuxWindows::new();
    let (id, mut session) = prepare(host, &mut windows, AuxKind::Settings);
    host.set_pause_owner(PauseOwner::User, true);
    let original = host.settings_snapshot().clone();
    let language = original.shared.general.language;
    let policy = controls(&session)
        .into_iter()
        .find(|c| c.label == text(language, UiText::SaveStoragePolicy))
        .unwrap();
    let ControlKind::Choice { choices, .. } = policy.kind else {
        panic!("storage choices")
    };
    let custom = choices
        .iter()
        .position(|label| label == text(language, UiText::CustomDirectory))
        .unwrap();
    session.dispatch(ControlEvent::new(policy.id, ControlInput::Select(custom)));
    submit(&mut session);
    assert!(session.screen().message.is_some());
    assert!(!session.mailbox().has_pending());
    assert!(!session.mailbox().close_requested());
    let (_, result) = host.close_widget_window(&mut windows, id, None).unwrap();
    assert!(result.is_ok());
    assert_eq!(host.settings_snapshot(), &original);
    assert!(!host.owns_pause(PauseOwner::Settings));
    assert!(host.owns_pause(PauseOwner::User));
    assert!(!windows.contains(AuxKind::Settings));
}
