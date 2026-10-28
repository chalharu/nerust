//! History exercises the same controls binding, not a second toolbar/program.
use super::*;
use crate::aux::widgets::controls::build_screen;
use nerust_settings_core::{
    i18n::{UiText, text},
    model::language::AppLanguage,
};
use wakui::control::{ControlKind, Layout, LayoutKind};

fn fixture() -> (HostState, AuxId, WidgetSession) {
    let mut host = TestHost::new().host;
    host.report_host_error("history test error".into());
    let (id, session) = prepare(&mut host, AuxKind::History);
    (host, id, session)
}

fn notes(session: &WidgetSession) -> Vec<String> {
    controls(&session.screen())
        .into_iter()
        .filter_map(|c| match c.kind {
            ControlKind::Note { text } => Some(text),
            _ => None,
        })
        .collect()
}

fn filter(session: &mut WidgetSession, index: usize) {
    let control = controls(&session.screen())
        .into_iter()
        .find(|c| matches!(c.kind, ControlKind::Choice { .. }))
        .unwrap();
    session.dispatch(Message::new(control.id, ControlInput::Select(index)));
}

#[test]
fn outer_history_window_uses_common_title_and_controls_pipeline() {
    use iced_winit::program::Program;
    let (_, _, session) = fixture();
    assert!(
        notes(&session)
            .iter()
            .any(|t| t.contains("history test error"))
    );
    let program = WindowProgram {
        window: session.window(),
        program: ControlsBinding::new(session),
    };
    let (state, _) = program.boot();
    let id = iced::window::Id::unique();
    assert_eq!(program.title(&state, id), state.borrow().screen().title);
    drop(program.view(&state, id));
}

#[test]
fn history_view_builds_empty_and_filtered() {
    let mut host = TestHost::new().host;
    let (_, mut session) = prepare(&mut host, AuxKind::History);
    drop(build_screen(&session.screen()));
    filter(&mut session, 1);
    assert!(
        notes(&session)
            .iter()
            .any(|t| t.contains("No notifications"))
    );
    drop(build_screen(&session.screen()));
}

#[test]
fn history_sync_sets_lines_language_and_preserves_filter() {
    let (mut host, _, mut session) = fixture();
    filter(&mut session, 3);
    let english = session.screen().title;
    let (settings_id, mut settings) = prepare(&mut host, AuxKind::Settings);
    let language = find(
        &settings.screen(),
        text(AppLanguage::English, UiText::Language),
    );
    let ControlKind::Choice { choices, .. } = language.kind else {
        unreachable!()
    };
    let japanese = choices.iter().position(|c| c.contains("日本")).unwrap();
    settings.dispatch(Message::new(language.id, ControlInput::Select(japanese)));
    let submit = controls(&settings.screen())
        .into_iter()
        .find(|c| c.display == wakui::control::ControlDisplay::Primary)
        .unwrap();
    settings.dispatch(Message::new(submit.id, ControlInput::Activate));
    host.finish_widget_window(settings_id, true);
    host.refresh_widget_session(&mut session);
    assert_ne!(session.screen().title, english);
    assert!(controls(&session.screen()).iter().any(|c| matches!(
        c.kind,
        ControlKind::Choice {
            selected: Some(3),
            ..
        }
    )));
    assert!(
        notes(&session)
            .iter()
            .any(|t| t.contains("history test error"))
    );
}

#[test]
fn history_view_builds_without_a_window() {
    let (mut host, _, mut session) = fixture();
    let binding = ControlsBinding::new(session);
    let (state, _) = binding.boot();
    drop(binding.view(&state, iced::window::Id::unique()));
    host.report_host_error("new history row".into());
    host.refresh_widget_session(&mut state.borrow_mut());
    session = Rc::try_unwrap(state).ok().unwrap().into_inner();
    assert!(
        notes(&session)
            .iter()
            .any(|t| t.contains("new history row"))
    );
    filter(&mut session, 3);
    drop(build_screen(&session.screen()));
}

#[test]
fn history_clear_applies_without_close_and_refreshes_same_filter() {
    let (mut host, id, mut session) = fixture();
    filter(&mut session, 3);
    activate(&mut session, "Clear History");
    assert!(session.mailbox().has_pending());
    assert!(!session.mailbox().close_requested());
    assert!(!host.finish_widget_window(id, false));
    assert!(!session.mailbox().has_pending());
    host.refresh_widget_session(&mut session);
    assert!(
        !notes(&session)
            .iter()
            .any(|t| t.contains("history test error"))
    );
    assert!(controls(&session.screen()).iter().any(|c| matches!(
        c.kind,
        ControlKind::Choice {
            selected: Some(3),
            ..
        }
    )));
    assert!(
        host.prepare_widget_session(id).is_none(),
        "original reservation/session survives clear"
    );
}

#[test]
fn sync_consumes_refresh_flag_not_close_flag() {
    let (mut host, _, mut session) = fixture();
    let mailbox = session.mailbox();
    mailbox.request_close();
    mailbox.invalidate();
    host.report_host_error("latest refresh".into());
    host.refresh_widget_session(&mut session);
    assert!(mailbox.take_invalidated());
    assert!(!mailbox.take_invalidated());
    assert!(mailbox.close_requested());
    assert!(notes(&session).iter().any(|t| t.contains("latest refresh")));
}

#[test]
fn merged_history_line_carries_single_count_suffix() {
    use nerust_gui_shell::session::{
        notification::{Message, NotificationLevel, UiNotification},
        osd::{history_lines, history_row_view},
    };
    use wakui::control::Node;
    let (_, _, session) = fixture();
    for language in [AppLanguage::English, AppLanguage::Japanese] {
        let notification = UiNotification {
            level: NotificationLevel::Info,
            message: Message::paused(),
            count: 3,
            seq: 0,
            at: std::time::UNIX_EPOCH,
        };
        let rows = history_lines(std::iter::once(&notification), language);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].count, 3);
        assert!(rows[0].text.ends_with("×3"));
        assert_eq!(rows[0].text.matches('×').count(), 1);
        let row = history_row_view(&rows[0], language);
        assert!(row.message.ends_with("×3"));
        assert_eq!(row.message.matches('×').count(), 1);

        // The native interpreter receives only the owned cell text: it must
        // not append the count again or require a History-specific binding.
        let mut screen = session.screen();
        let mut cell = controls(&screen)
            .into_iter()
            .find(|c| matches!(c.kind, ControlKind::Note { .. }))
            .unwrap();
        cell.kind = ControlKind::Note { text: row.message };
        screen.pages[0].body.children = vec![Node::Control(cell.clone())];
        assert_eq!(screen.control(cell.id).unwrap(), &cell);
        drop(build_screen(&screen));
    }
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
        fn begin_layout(&mut self, _: &Layout, kind: LayoutKind) {
            self.depth += 1;
            self.tables += usize::from(matches!(kind, LayoutKind::Table));
            self.rows += usize::from(matches!(kind, LayoutKind::Row));
        }
        fn end_layout(&mut self, _: &Layout, _: LayoutKind) {
            self.depth -= 1;
        }
        fn control(&mut self, _: &Control) {
            self.controls += 1;
        }
    }
    let (_, _, mut session) = fixture();
    for index in [0, 3] {
        filter(&mut session, index);
        let screen = session.screen();
        let mut probe = Probe {
            depth: 0,
            tables: 0,
            rows: 0,
            controls: 0,
        };
        screen.visit(&mut probe);
        assert_eq!(probe.depth, 0);
        assert_eq!(probe.tables, 1);
        assert!(probe.rows >= 3, "header, actual row, footer");
        assert!(probe.controls >= 11);
        drop(build_screen(&screen));
    }
}
