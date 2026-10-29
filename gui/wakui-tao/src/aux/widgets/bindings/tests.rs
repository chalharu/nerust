use crate::binding::{ControlsBinding, Message, keyboard_key_from_physical};
use crate::handler::WidgetBinding;
use crate::{
    runtime::host::{HostState, TestHost},
    testkit::TestAux,
    window::WindowProgram,
};
use std::rc::Rc;
use wakui_core::Key;
use wakui_core::chrome::aux::{AuxId, OpenRequest};
use wakui_core::scene::control::{
    Control, ControlEvent, ControlInput, ControlScreen, ControlVisitor,
};

mod log;

pub(crate) fn prepare(host: &mut HostState<TestAux>, kind: TestAux) -> AuxId {
    let Some(OpenRequest::Create(id)) = host.reserve_aux_window(kind) else {
        panic!("reservation")
    };
    assert!(host.prepare_widget(id), "prepared widget");
    id
}

pub(crate) fn screen_of(host: &HostState<TestAux>, id: AuxId) -> ControlScreen {
    host.core.widget_screen(id).expect("widget screen")
}

pub(crate) fn mailbox_of(
    host: &HostState<TestAux>,
    id: AuxId,
) -> wakui_core::chrome::mailbox::WindowMailbox {
    host.core.widget_mailbox(id).expect("widget mailbox")
}

pub(crate) fn dispatch(
    host: &mut HostState<TestAux>,
    id: AuxId,
    event: ControlEvent,
) -> Vec<wakui_core::scene::control::ControlEffect> {
    host.core.widget_dispatch(id, event)
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

pub(crate) fn activate(host: &mut HostState<TestAux>, id: AuxId, label: &str) {
    let screen = screen_of(host, id);
    let control = find(&screen, label);
    let _ = dispatch(host, id, Message::new(control.id, ControlInput::Activate));
}

#[test]
fn common_window_program_moves_the_prepared_controller_and_binds_latest_screen() {
    use iced_winit::program::Program;
    let mut host = TestHost::new().host;
    let id = prepare(&mut host, TestAux::Alpha);
    let before = screen_of(&host, id);
    let page = &before.pages[3];
    let _ = dispatch(
        &mut host,
        id,
        Message::new(page.select.id, ControlInput::Activate),
    );
    let after = screen_of(&host, id);
    let (binding, shared_screen, _queue) = ControlsBinding::new(after.clone());
    assert!(binding.boot_pending());
    let window = host.aux.window(id).expect("reserved window").clone();
    let program = WindowProgram {
        window,
        program: binding,
    };
    let (mut state, _) = program.boot();
    assert!(!program.program.boot_pending());
    assert!(Rc::ptr_eq(&shared_screen, &state.screen));
    assert_eq!(
        state
            .screen
            .borrow()
            .pages
            .iter()
            .find(|p| p.selected)
            .unwrap()
            .id,
        page.id
    );
    let control = find(&state.screen.borrow(), "Toggle A");
    drop(program.update(
        &mut state,
        Message::new(control.id, ControlInput::Bool(true)),
    ));
    // Update queues for the host drain; the pushed screen still shows the old value.
    assert!(state.screen.borrow().control(control.id).is_some());
    let id = iced::window::Id::unique();
    assert_eq!(program.title(&state, id), state.screen.borrow().title);
    drop(program.view(&state, id));
}

#[test]
fn prepared_window_contains_the_controller_moved_into_boot() {
    let mut host = TestHost::new().host;
    let id = prepare(&mut host, TestAux::Alpha);
    let screen = screen_of(&host, id);
    let page = screen.pages[4].clone();
    let _ = dispatch(
        &mut host,
        id,
        Message::new(page.select.id, ControlInput::Activate),
    );
    let updated = screen_of(&host, id);
    let binding = ControlsBinding::new(updated.clone());
    let (state, _) = binding.0.boot();
    assert!(!binding.0.boot_pending());
    assert_eq!(*state.screen.borrow(), updated);
    assert!(matches!(
        host.aux.window(id).expect("window").contents.contents,
        wakui_core::chrome::window::WidgetContents::Controls(_)
    ));
}

#[test]
fn status_area_message_reaches_viewmodel() {
    let mut host = TestHost::new().host;
    let id = prepare(&mut host, TestAux::Alpha);
    for (label, input) in [
        ("Toggle B", ControlInput::Bool(true)),
        ("Choice D", ControlInput::Select(0)),
        ("Choice E", ControlInput::Select(2)),
        ("Choice F", ControlInput::Select(1)),
    ] {
        let screen = screen_of(&host, id);
        let c = find(&screen, label);
        let _ = dispatch(&mut host, id, Message::new(c.id, input));
    }
    let screen = screen_of(&host, id);
    let c = find(&screen, "Choice F");
    assert_eq!(
        c.downcast::<wakui_core::scene::leaf::ChoiceLeaf>()
            .unwrap()
            .selected,
        Some(1)
    );
    drop(crate::controls::build_screen(&screen));
}

#[test]
fn program_update_and_error_paths_stay_headless() {
    let mut host = TestHost::new().host;
    let id = prepare(&mut host, TestAux::Alpha);
    let screen = screen_of(&host, id);
    let (binding, _, _) = ControlsBinding::new(screen);
    let (mut state, _) = binding.boot();
    // Updates queue for the host drain; every page stays addressable.
    let pages = state.screen.borrow().pages.clone();
    for page in pages {
        drop(binding.update(
            &mut state,
            Message::new(page.select.id, ControlInput::Activate),
        ));
    }
    assert!(!state.queue.borrow().is_empty());
    drop(binding.view(&state, iced::window::Id::unique()));
}

#[test]
fn program_boot_and_chrome_are_headless() {
    for kind in [TestAux::Alpha, TestAux::Beta] {
        let mut host = TestHost::new().host;
        let id = prepare(&mut host, kind);
        let screen = screen_of(&host, id);
        let mailbox = mailbox_of(&host, id);
        let (binding, state_screen, _) = ControlsBinding::new(screen);
        let _ = binding.settings();
        let (state, _) = binding.boot();
        assert!(Rc::ptr_eq(&state_screen, &state.screen));
        mailbox.request_close();
        assert!(mailbox.close_requested());
        assert!(binding.theme(&state, iced::window::Id::unique()).is_none());
        assert!(binding.presentation(&state).is_none());
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
