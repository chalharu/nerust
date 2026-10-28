use super::*;
use crate::{
    aux::window::WindowProgram,
    runtime::host::{HostState, TestHost},
};
use nerust_frontend_host::aux::{AuxId, OpenRequest};
use wakui::control::{Control, ControlInput, ControlVisitor};

mod history;

pub(crate) fn prepare(host: &mut HostState, kind: AuxKind) -> (AuxId, WidgetSession) {
    let Some(OpenRequest::Create(id)) = host.reserve_aux_window(kind) else {
        panic!("reservation")
    };
    (
        id,
        host.prepare_widget_session(id).expect("prepared session"),
    )
}

pub(crate) fn controls(screen: &ControlScreen) -> Vec<Control> {
    struct Collect(Vec<Control>);
    impl ControlVisitor for Collect {
        fn control(&mut self, control: &Control) {
            self.0.push(control.clone());
        }
    }
    let mut collect = Collect(Vec::new());
    screen.visit(&mut collect);
    collect.0
}

pub(crate) fn find(screen: &ControlScreen, label: &str) -> Control {
    controls(screen)
        .into_iter()
        .find(|c| c.label == label || c.metadata.accessible_label == label)
        .unwrap_or_else(|| {
            panic!(
                "control {label:?} not found: {:?}",
                controls(screen)
                    .iter()
                    .map(|c| &c.label)
                    .collect::<Vec<_>>()
            )
        })
}

pub(crate) fn activate(session: &mut WidgetSession, label: &str) {
    let control = find(&session.screen(), label);
    session.dispatch(Message::new(control.id, ControlInput::Activate));
}

#[test]
fn common_window_program_moves_the_prepared_controller_and_binds_latest_screen() {
    use iced_winit::program::Program;
    let mut host = TestHost::new().host;
    let (_, mut session) = prepare(&mut host, AuxKind::Settings);
    let before = session.screen();
    let page = &before.pages[3];
    session.dispatch(Message::new(page.select.id, ControlInput::Activate));
    let window = session.window();
    let binding = ControlsBinding::new(session);
    let shared = binding.session();
    let program = WindowProgram {
        window,
        program: binding,
    };
    let (mut state, _) = program.boot();
    assert!(program.program.initial_state.borrow().is_none());
    assert!(Rc::ptr_eq(&shared, &state));
    assert_eq!(
        state
            .borrow()
            .screen()
            .pages
            .iter()
            .find(|p| p.selected)
            .unwrap()
            .id,
        page.id
    );
    let control = find(&state.borrow().screen(), "Mute");
    drop(program.update(
        &mut state,
        Message::new(control.id, ControlInput::Bool(true)),
    ));
    assert!(matches!(
        state.borrow().screen().control(control.id).unwrap().kind,
        wakui::control::ControlKind::Toggle { value: true }
    ));
    let id = iced::window::Id::unique();
    assert_eq!(program.title(&state, id), state.borrow().screen().title);
    drop(program.view(&state, id));
}

#[test]
fn prepared_window_contains_the_controller_moved_into_boot() {
    let mut host = TestHost::new().host;
    let (_, mut session) = prepare(&mut host, AuxKind::Settings);
    let page = session.screen().pages[4].clone();
    session.dispatch(Message::new(page.select.id, ControlInput::Activate));
    let screen = session.screen();
    let binding = ControlsBinding::new(session);
    let (state, _) = binding.boot();
    assert!(binding.initial_state.borrow().is_none());
    assert_eq!(state.borrow().screen(), screen);
    assert!(matches!(
        state.borrow().window().contents.contents,
        nerust_frontend_host::window::WidgetContents::Controls(_)
    ));
}

#[test]
fn status_area_message_reaches_viewmodel() {
    use nerust_settings_core::{
        i18n::{UiText, text},
        model::language::AppLanguage,
    };
    let mut host = TestHost::new().host;
    let (_, mut session) = prepare(&mut host, AuxKind::Settings);
    for (key, input) in [
        (UiText::NotificationStatus, ControlInput::Bool(true)),
        (UiText::NotificationPosition, ControlInput::Select(0)),
        (UiText::NotificationStatusPosition, ControlInput::Select(2)),
        (UiText::NotificationStatusAreaMode, ControlInput::Select(1)),
    ] {
        let c = find(&session.screen(), text(AppLanguage::English, key));
        session.dispatch(Message::new(c.id, input));
    }
    let c = find(
        &session.screen(),
        text(AppLanguage::English, UiText::NotificationStatusAreaMode),
    );
    assert!(matches!(
        c.kind,
        wakui::control::ControlKind::Choice {
            selected: Some(1),
            ..
        }
    ));
    drop(super::super::controls::build_screen(&session.screen()));
}

#[test]
fn program_update_and_error_paths_stay_headless() {
    let mut host = TestHost::new().host;
    let (_, session) = prepare(&mut host, AuxKind::Settings);
    let binding = ControlsBinding::new(session);
    let (mut state, _) = binding.boot();
    let pages = state.borrow().screen().pages;
    for page in pages {
        drop(binding.update(
            &mut state,
            Message::new(page.select.id, ControlInput::Activate),
        ));
        assert!(
            state
                .borrow()
                .screen()
                .pages
                .iter()
                .find(|p| p.id == page.id)
                .unwrap()
                .selected
        );
    }
    drop(binding.view(&state, iced::window::Id::unique()));
}

#[test]
fn program_boot_and_chrome_are_headless() {
    for kind in [AuxKind::Settings, AuxKind::History] {
        let mut host = TestHost::new().host;
        let (_, session) = prepare(&mut host, kind);
        let mailbox = session.mailbox();
        let binding = ControlsBinding::new(session);
        let _ = binding.settings();
        let (state, _) = binding.boot();
        mailbox.request_close();
        assert!(state.borrow().mailbox().close_requested());
        assert!(binding.theme(&state, iced::window::Id::unique()).is_none());
        assert_eq!(
            binding.presentation(&state).unwrap().title,
            state.borrow().screen().title
        );
        drop(binding.view(&state, iced::window::Id::unique()));
    }
}

#[test]
fn physical_key_mapping_matches_tao_bindings() {
    use iced::keyboard::key::{Code, Physical};
    for (code, key) in [
        (Code::KeyZ, Key::KeyZ),
        (Code::ArrowLeft, Key::ArrowLeft),
        (Code::F11, Key::F11),
    ] {
        assert_eq!(keyboard_key_from_physical(Physical::Code(code)), Some(key));
    }
}
