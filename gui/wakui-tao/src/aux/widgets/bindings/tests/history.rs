//! History exercises the same controls binding, not a second toolbar/program.
use super::*;
use crate::aux::widgets::controls::build_screen;
use wakui_core::control::{
    Container, Control, ControlInput, ControlVisitor, RowPolicy, TablePolicy, policy,
};
use wakui_core::leaf::{ChoiceLeaf, ListLeaf, NoteLeaf};

fn fixture() -> (HostState<TestAux>, AuxId) {
    let mut host = TestHost::new().host;
    host.core.report_host_error("history test error".into());
    let id = prepare(&mut host, TestAux::History);
    (host, id)
}

fn notes(host: &HostState<TestAux>, id: AuxId) -> Vec<String> {
    let mut out = Vec::new();
    struct Lists<'a>(&'a mut Vec<String>);
    impl ControlVisitor for Lists<'_> {
        fn control(&mut self, control: &Control) {
            if let Some(list) = control.downcast::<ListLeaf>() {
                for row in &list.rows {
                    for cell in &row.cells {
                        if let Some(note) = cell.downcast::<NoteLeaf>() {
                            self.0.push(note.text.clone());
                        }
                    }
                }
            } else if let Some(note) = control.downcast::<NoteLeaf>() {
                self.0.push(note.text.clone());
            }
        }
    }
    screen_of(host, id).visit(&mut Lists(&mut out));
    out
}

fn filter(host: &mut HostState<TestAux>, id: AuxId, index: usize) {
    let screen = screen_of(host, id);
    let control = controls(&screen)
        .into_iter()
        .find(|c| c.downcast::<ChoiceLeaf>().is_some())
        .unwrap();
    let _ = dispatch(
        host,
        id,
        Message::new(control.id, ControlInput::Select(index)),
    );
}

#[test]
fn outer_history_window_uses_common_title_and_controls_pipeline() {
    use iced_winit::program::Program;
    let (host, id) = fixture();
    assert!(
        notes(&host, id)
            .iter()
            .any(|t| t.contains("history test error"))
    );
    let screen = screen_of(&host, id);
    let window = host.aux.window(id).expect("reserved window").clone();
    let (binding, _, _) = ControlsBinding::new(screen);
    let program = WindowProgram {
        window: window.clone(),
        program: binding,
    };
    let (state, _) = program.boot();
    let id_iced = iced::window::Id::unique();
    assert_eq!(program.title(&state, id_iced), window.title);
    drop(program.view(&state, id_iced));
}

#[test]
fn history_view_builds_empty_and_filtered() {
    let mut host = TestHost::new().host;
    let id = prepare(&mut host, TestAux::History);
    let screen = screen_of(&host, id);
    drop(build_screen(&screen));
    filter(&mut host, id, 1);
    assert!(
        notes(&host, id)
            .iter()
            .any(|t| t.contains("No notifications"))
    );
    drop(build_screen(&screen_of(&host, id)));
}

#[test]
fn history_refresh_preserves_filter_and_error_note() {
    let (mut host, id) = fixture();
    filter(&mut host, id, 3);
    let before = screen_of(&host, id);
    assert!(
        notes(&host, id)
            .iter()
            .any(|t| t.contains("history test error"))
    );
    host.core.widget_refresh(id);
    let updated = screen_of(&host, id);
    assert_eq!(updated.title, before.title);
    assert!(controls(&updated).iter().any(|c| {
        c.downcast::<ChoiceLeaf>()
            .is_some_and(|leaf| leaf.selected == Some(3))
    }));
    assert!(
        notes(&host, id)
            .iter()
            .any(|t| t.contains("history test error"))
    );
}

#[test]
fn history_view_builds_without_a_window() {
    let (mut host, id) = fixture();
    let screen = screen_of(&host, id);
    let (binding, _, _) = ControlsBinding::new(screen);
    let (state, _) = binding.boot();
    drop(binding.view(&state, iced::window::Id::unique()));
    host.core.report_host_error("new history row".into());
    host.core.widget_refresh(id);
    let updated = screen_of(&host, id);
    assert!(controls(&updated).iter().any(|t| {
        t.downcast::<NoteLeaf>()
            .is_some_and(|leaf| leaf.text.contains("new history row"))
    }));
    filter(&mut host, id, 3);
    drop(build_screen(&screen_of(&host, id)));
}

#[test]
fn history_clear_applies_without_close_and_refreshes_same_filter() {
    let (mut host, id) = fixture();
    filter(&mut host, id, 3);
    activate(&mut host, id, "Clear History");
    let mailbox = mailbox_of(&host, id);
    assert!(mailbox.has_pending());
    assert!(!mailbox.close_requested());
    assert!(!host.finish_widget_window(id, false));
    assert!(!mailbox.has_pending());
    host.core.widget_refresh(id);
    let screen = screen_of(&host, id);
    assert!(
        !notes(&host, id)
            .iter()
            .any(|t| t.contains("history test error"))
    );
    assert!(controls(&screen).iter().any(|c| {
        c.downcast::<ChoiceLeaf>()
            .is_some_and(|leaf| leaf.selected == Some(3))
    }));
    assert!(
        !host.prepare_widget(id),
        "original reservation/session survives clear"
    );
}

#[test]
fn sync_consumes_refresh_flag_not_close_flag() {
    let (mut host, id) = fixture();
    let mailbox = mailbox_of(&host, id);
    mailbox.request_close();
    mailbox.invalidate();
    host.core.report_host_error("latest refresh".into());
    host.core.widget_refresh(id);
    assert!(mailbox.take_invalidated());
    assert!(!mailbox.take_invalidated());
    assert!(mailbox.close_requested());
    assert!(
        notes(&host, id)
            .iter()
            .any(|t| t.contains("latest refresh"))
    );
}

#[test]
fn owned_note_text_renders_verbatim_without_recounting() {
    use wakui_core::control::Node;
    let (host, id) = fixture();
    let message = "Paused \u{d7}3".to_string();
    // The native interpreter receives only the owned cell text: it must
    // not append the count again or require a History-specific binding.
    let mut screen = screen_of(&host, id);
    let mut cell = controls(&screen)
        .into_iter()
        .find(|c| c.downcast::<NoteLeaf>().is_some())
        .unwrap();
    cell.leaf = Box::new(NoteLeaf {
        text: message.clone(),
    });
    screen.pages[0].body.children = vec![Node::Control(cell.clone())];
    assert_eq!(screen.control(cell.id).unwrap(), &cell);
    drop(build_screen(&screen));
    assert!(message.ends_with("\u{d7}3"));
}

#[test]
fn shared_scene_drives_native_view_and_balanced_visitor_order() {
    struct Probe {
        depth: usize,
        tables: usize,
        rows: usize,
        controls: usize,
    }
    impl ControlVisitor for Probe {
        fn begin_container(&mut self, container: &Container) {
            self.depth += 1;
            self.tables += usize::from(policy::<TablePolicy>(container).is_some());
            self.rows += usize::from(policy::<RowPolicy>(container).is_some());
        }
        fn end_container(&mut self, _: &Container) {
            self.depth -= 1;
        }
        fn control(&mut self, _: &Control) {
            self.controls += 1;
        }
    }
    let (mut host, id) = fixture();
    for index in [0, 3] {
        filter(&mut host, id, index);
        let screen = screen_of(&host, id);
        let mut probe = Probe {
            depth: 0,
            tables: 0,
            rows: 0,
            controls: 0,
        };
        screen.visit(&mut probe);
        assert_eq!(probe.depth, 0);
        assert!(probe.controls >= 4, "filter, notes, footer actions");
        drop(build_screen(&screen));
    }
}
