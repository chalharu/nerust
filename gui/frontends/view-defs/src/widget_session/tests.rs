use super::*;
use wakui::{
    binding::PickerSink as _,
    control::{
        Control, ControlInput, ControlKind, ControlVisitor, IdRegistry, Layout, LayoutKind, Page,
        Tab, Tabs,
    },
};

use crate::{
    settings::{SettingsControlId, SettingsSystemId},
    window::{WidgetContents, WindowVisitor},
};
use nerust_core_traits::audio::AudioBackendRegistry;
use nerust_gui_shell::registry::SystemRegistry;
use nerust_settings_core::{
    editor::CaptureTarget,
    model::{input::ShortcutAction, shared::StoragePolicy},
};

fn settings(nes: bool) -> WidgetSession {
    let mut snapshot = SettingsSnapshot {
        shared: Default::default(),
        local: Default::default(),
        app_state: Default::default(),
    };
    snapshot.shared.input.shortcuts.keyboard =
        nerust_settings_core::model::input::default_shortcut_bindings();
    snapshot.shared.general.language = AppLanguage::English;
    WidgetSession::from_settings(SettingsController::new(
        &snapshot,
        Arc::new(SystemRegistry::new(if nes {
            vec![nerust_nes_factory::NesFactory::boxed().into()]
        } else {
            vec![]
        })),
        Arc::new(AudioBackendRegistry::new()),
    ))
}

fn token(session: &WidgetSession, domain: SettingsControlId) -> ControlId {
    let State::Settings(controller) = &session.state else {
        panic!()
    };
    session
        .scene
        .borrow_mut()
        .settings(&controller.screen())
        .1
        .into_iter()
        .find(|(_, id)| *id == domain)
        .unwrap()
        .0
}

fn send(session: &mut WidgetSession, id: ControlId, input: ControlInput) {
    assert!(session.dispatch(ControlEvent::new(id, input)).is_empty());
}

#[derive(Default)]
struct Inventory(Vec<Control>);
impl ControlVisitor for Inventory {
    fn begin_page(&mut self, p: &Page) {
        self.0.push(p.select.clone());
    }
    fn begin_tab(&mut self, t: &Tab) {
        self.0.push(t.select.clone());
    }
    fn control(&mut self, c: &Control) {
        self.0.push(c.clone());
    }
}

fn inventory(screen: &ControlScreen) -> Vec<Control> {
    let mut visitor = Inventory::default();
    screen.visit(&mut visitor);
    visitor.0
}

#[test]
fn both_domains_use_the_same_real_scene_interpreter_and_input() {
    let mut session = settings(false);
    let screen = session.screen();
    assert_eq!(screen.pages.len(), 6);
    let volume = token(&session, SettingsControlId::Volume);
    send(&mut session, volume, ControlInput::Number(37));
    assert!(matches!(
        session.screen().control(volume).unwrap().kind,
        ControlKind::Number { value: 37, .. }
    ));

    let mut history = WidgetSession::history(Arc::new([]), AppLanguage::English);
    let filter = history.scene.borrow_mut().history_filter();
    send(&mut history, filter, ControlInput::Select(3));
    assert!(matches!(
        history.screen().control(filter).unwrap().kind,
        ControlKind::Choice {
            selected: Some(3),
            ..
        }
    ));
    let clear = history.scene.borrow_mut().history_clear();
    send(&mut history, clear, ControlInput::Activate);
    assert!(history.mailbox().has_pending());
    assert!(matches!(
        take_submission(&history.mailbox()).unwrap().take(),
        Some(Pending::ClearHistory)
    ));
    let close = history.scene.borrow_mut().history_close();
    send(&mut history, close, ControlInput::Activate);
    assert!(history.mailbox().close_requested());
    for session in [session, history] {
        let WidgetContents::Controls(screen) = session.window().contents.contents else {
            panic!("both domains must use the generic native interpreter")
        };
        let controls = inventory(&screen);
        for (index, control) in controls.iter().enumerate() {
            assert!(screen.control(control.id).is_some());
            assert!(controls[..index].iter().all(|c| c.id != control.id));
        }
    }
}

#[test]
fn equality_registry_survives_reorder_removal_and_never_aliases_another_owner() {
    #[derive(PartialEq, Eq)]
    struct Domain(&'static str);
    let mut ids = IdRegistry::default();
    let a = ids.id(Domain("a"));
    let b = ids.id(Domain("b"));
    assert_eq!(ids.id(Domain("b")), b);
    let c = ids.id(Domain("c"));
    assert_ne!(a, c);
    assert_ne!(b, c);
    assert_eq!(ids.id(Domain("a")), a);
    assert_ne!(IdRegistry::default().id(Domain("a")), a);
}

#[test]
fn history_controls_filter_actual_rows_and_reject_invalid_choice_indices() {
    use nerust_gui_shell::session::notification::NotificationLevel;
    let lines = [
        NotificationLevel::Info,
        NotificationLevel::Warning,
        NotificationLevel::Error,
    ]
    .map(|level| HistoryLine {
        datetime: "2026-10-27 12:00:00".into(),
        level,
        module: "test::source",
        text: format!("row-{level:?}"),
        count: 1,
    });
    let mut session = WidgetSession::history(lines.into(), AppLanguage::English);
    let filter = session.scene.borrow_mut().history_filter();
    let rows = |session: &WidgetSession| {
        inventory(&session.screen())
            .into_iter()
            .filter_map(|c| match c.kind {
                ControlKind::Note { text } if text.starts_with("row-") => Some(text),
                _ => None,
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(rows(&session).len(), 3);
    send(&mut session, filter, ControlInput::Select(2));
    assert_eq!(rows(&session), ["row-Warning", "row-Error"]);
    send(&mut session, filter, ControlInput::Select(usize::MAX));
    assert_eq!(rows(&session).len(), 2);
    send(&mut session, filter, ControlInput::Select(3));
    assert_eq!(rows(&session), ["row-Error"]);
    assert_eq!(session.scene.borrow_mut().history_filter(), filter);
}

#[test]
fn clearing_mailbox_error_invalidates_even_when_the_input_value_is_unchanged() {
    let mut session = settings(false);
    let volume = token(&session, SettingsControlId::Volume);
    session.mailbox.apply_failed("save error".into());
    assert!(session.mailbox.take_invalidated());
    send(&mut session, volume, ControlInput::Number(100));
    assert!(session.screen().message.is_none());
    assert!(session.mailbox.take_invalidated());
}

#[test]
fn latest_topology_and_navigation_bind_old_callbacks_without_reusing_tokens() {
    let mut session = settings(true);
    let old = session.screen();
    let volume = token(&session, SettingsControlId::Volume);
    let State::Settings(controller) = &session.state else {
        panic!()
    };
    let view = controller.vm.inputs()[0].view.get();
    let slot = &view.slots[0];
    let domain = SettingsControlId::ControllerSlot {
        system: SettingsSystemId(view.system_id.to_string()),
        slot: slot.slot_id,
    };
    let slot_id = token(&session, domain.clone());
    let none = slot.choices.iter().position(|c| c.value.is_none()).unwrap();
    if let State::Settings(controller) = &mut session.state {
        controller.input_tab = Some(crate::settings::InputTab::Shortcuts);
    }
    send(&mut session, slot_id, ControlInput::Select(none));
    assert_eq!(token(&session, domain), slot_id);
    assert_eq!(token(&session, SettingsControlId::Volume), volume);
    if let State::Settings(controller) = &session.state {
        assert_eq!(
            controller.input_tab,
            Some(crate::settings::InputTab::System(0))
        );
        assert!(
            controller.vm.inputs()[0].view.get().slots[0]
                .selected_profile_id
                .is_none()
        );
    }
    let latest = session.screen();
    let removed = inventory(&old)
        .into_iter()
        .filter(|c| latest.control(c.id).is_none())
        .collect::<Vec<_>>();
    assert!(
        !removed.is_empty(),
        "removing the controller changes real NES topology"
    );
    for control in removed {
        assert!(
            session
                .dispatch(ControlEvent::new(control.id, ControlInput::Activate))
                .is_empty()
        );
        assert!(session.screen().capture.target.is_none());
    }
    send(&mut session, volume, ControlInput::Number(29));
    assert!(matches!(
        session.screen().control(volume).unwrap().kind,
        ControlKind::Number { value: 29, .. }
    ));
}

#[test]
fn legacy_positional_empty_tabs_and_conflicts_use_semantic_tokens_after_reorder() {
    use crate::settings::{SettingsControlKind, SettingsLayoutId, SettingsNode, SettingsTabId};
    let session = settings(false);
    let State::Settings(controller) = &session.state else {
        panic!()
    };
    let mut typed = controller.screen();
    let SettingsNode::Tabs(tabs) = &mut typed.pages[1].body.children[0] else {
        panic!()
    };
    let template = tabs.tabs[0].clone();
    let system = SettingsSystemId("semantic-system".into());
    tabs.tabs = ["empty-a", "empty-b"]
        .into_iter()
        .enumerate()
        .map(|(index, label)| {
            let mut tab = template.clone();
            tab.id = SettingsTabId::EmptyAttachment {
                system: system.clone(),
                index,
            };
            tab.label = label.into();
            tab.select.id = SettingsControlId::Tab(tab.id.clone());
            tab.select.label = label.into();
            tab.body.id = SettingsLayoutId::Tab(tab.id.clone());
            tab.body.children.clear();
            tab
        })
        .collect();
    let template = typed
        .control(&SettingsControlId::NoSystems)
        .unwrap()
        .clone();
    for (index, label) in ["conflict-a", "conflict-b"].into_iter().enumerate() {
        let mut control = template.clone();
        control.id = SettingsControlId::Conflict {
            system: system.clone(),
            index,
        };
        control.label = label.into();
        control.kind = SettingsControlKind::Note { text: label.into() };
        typed.pages[0]
            .body
            .children
            .push(SettingsNode::Control(control));
    }
    let mut scene = scene::SceneBuilder::default();
    let (old, _) = scene.settings(&typed);
    let ids = inventory(&old)
        .into_iter()
        .filter(|c| c.label.starts_with("empty-") || c.label.starts_with("conflict-"))
        .collect::<Vec<_>>();
    assert_eq!(ids.len(), 4);
    let SettingsNode::Tabs(tabs) = &mut typed.pages[1].body.children[0] else {
        panic!()
    };
    tabs.tabs.reverse();
    for (index, tab) in tabs.tabs.iter_mut().enumerate() {
        tab.id = SettingsTabId::EmptyAttachment {
            system: system.clone(),
            index,
        };
        tab.select.id = SettingsControlId::Tab(tab.id.clone());
        tab.body.id = SettingsLayoutId::Tab(tab.id.clone());
    }
    typed.pages[0].body.children.reverse();
    let mut index = 0;
    for node in &mut typed.pages[0].body.children {
        if let SettingsNode::Control(c) = node
            && matches!(c.id, SettingsControlId::Conflict { .. })
        {
            c.id = SettingsControlId::Conflict {
                system: system.clone(),
                index,
            };
            index += 1;
        }
    }
    let (latest, bindings) = scene.settings(&typed);
    for control in ids {
        assert_eq!(latest.control(control.id).unwrap().label, control.label);
        let domain = &bindings.iter().find(|(id, _)| *id == control.id).unwrap().1;
        let old_index = if control.label.ends_with('a') { 0 } else { 1 };
        match domain {
            SettingsControlId::Tab(SettingsTabId::EmptyAttachment { index, .. })
            | SettingsControlId::Conflict { index, .. } => assert_ne!(*index, old_index),
            _ => panic!("retained token must bind the latest typed identity"),
        }
    }
}

#[test]
fn typed_ranges_latest_enable_visibility_and_validation_are_host_owned() {
    let mut session = settings(false);
    let volume = token(&session, SettingsControlId::Volume);
    for input in [
        ControlInput::Number(101),
        ControlInput::Number(u32::MAX),
        ControlInput::Bool(true),
    ] {
        send(&mut session, volume, input);
    }
    assert!(matches!(
        session.screen().control(volume).unwrap().kind,
        ControlKind::Number { value: 100, .. }
    ));
    let enabled = token(&session, SettingsControlId::InfoEnabled);
    let duration = token(&session, SettingsControlId::InfoDuration);
    send(&mut session, enabled, ControlInput::Bool(false));
    send(&mut session, duration, ControlInput::Number(2000));
    assert!(!session.screen().control(duration).unwrap().metadata.enabled);
    let storage = token(&session, SettingsControlId::StorageDirectory);
    send(&mut session, storage, ControlInput::Text("ignored".into()));
    assert!(!session.screen().control(storage).unwrap().metadata.visible);
    if let State::Settings(controller) = &session.state {
        controller
            .vm
            .general
            .set_storage_policy(StoragePolicy::CustomDirectory)
            .unwrap();
    }
    let submit = token(&session, SettingsControlId::Submit);
    send(&mut session, submit, ControlInput::Activate);
    assert!(!session.mailbox().has_pending());
    assert!(!session.mailbox().close_requested());
    assert!(session.screen().message.is_some());
    send(&mut session, storage, ControlInput::Text("/tmp".into()));
    send(&mut session, submit, ControlInput::Activate);
    assert!(session.mailbox().has_pending());
}

#[test]
fn generic_picker_and_capture_results_need_no_domain_dto() {
    let mut session = settings(false);
    if let State::Settings(controller) = &session.state {
        controller
            .vm
            .general
            .set_storage_policy(StoragePolicy::CustomDirectory)
            .unwrap();
    }
    let directory = token(&session, SettingsControlId::StorageDirectory);
    let effects = session.dispatch(ControlEvent::new(directory, ControlInput::Activate));
    let [
        ControlEffect::PickDirectory {
            title,
            target,
            request,
        },
    ] = effects.as_slice()
    else {
        panic!()
    };
    assert!(!title.is_empty());
    assert_eq!(*target, directory);
    session.submit(directory, *request, Some("/tmp".into()));
    assert!(matches!(&session.screen().control(directory).unwrap().kind,
        ControlKind::Text { value, .. } if value == "/tmp"));
    session.submit(directory, *request, Some("/stale".into()));
    assert!(matches!(&session.screen().control(directory).unwrap().kind,
        ControlKind::Text { value, .. } if value == "/tmp"));
    let binding = token(
        &session,
        SettingsControlId::Binding(CaptureTarget::Shortcut(ShortcutAction::TogglePause)),
    );
    send(&mut session, binding, ControlInput::Activate);
    assert_eq!(session.screen().capture.target, Some(binding));
    assert!(
        session
            .dispatch(ControlEvent::capture_key(wakui::Key::Space))
            .is_empty()
    );
    assert!(session.screen().capture.target.is_none());
    assert!(matches!(&session.screen().control(binding).unwrap().kind,
        ControlKind::Binding { capturing: false, text, .. } if text.contains("Space")));
}

#[test]
fn owned_generic_visitors_reenter_with_no_controller_borrow() {
    struct Reenter<'a> {
        session: &'a RefCell<WidgetSession>,
        volume: ControlId,
        stack: Vec<wakui::control::Id>,
        count: usize,
    }
    impl ControlVisitor for Reenter<'_> {
        fn begin_page(&mut self, page: &Page) {
            self.stack.push(page.id);
        }
        fn end_page(&mut self, page: &Page) {
            assert_eq!(self.stack.pop(), Some(page.id));
        }
        fn begin_layout(&mut self, layout: &Layout, _: LayoutKind) {
            self.stack.push(layout.id);
        }
        fn end_layout(&mut self, layout: &Layout, _: LayoutKind) {
            assert_eq!(self.stack.pop(), Some(layout.id));
        }
        fn begin_tabs(&mut self, tabs: &Tabs) {
            self.stack.push(tabs.id);
        }
        fn end_tabs(&mut self, tabs: &Tabs) {
            assert_eq!(self.stack.pop(), Some(tabs.id));
        }
        fn begin_tab(&mut self, tab: &Tab) {
            self.stack.push(tab.id);
        }
        fn end_tab(&mut self, tab: &Tab) {
            assert_eq!(self.stack.pop(), Some(tab.id));
        }
        fn control(&mut self, _: &Control) {
            self.count += 1;
            send(
                &mut self.session.borrow_mut(),
                self.volume,
                ControlInput::Number(21),
            );
            assert_eq!(self.session.borrow().screen().pages.len(), 6);
        }
    }
    impl WindowVisitor for Reenter<'_> {
        fn visit_controls(&mut self, screen: &ControlScreen) {
            screen.visit(self as &mut dyn ControlVisitor);
        }
    }
    let session = RefCell::new(settings(true));
    let volume = token(&session.borrow(), SettingsControlId::Volume);
    let window = session.borrow().window();
    let mut visitor = Reenter {
        session: &session,
        volume,
        stack: Vec::new(),
        count: 0,
    };
    window.visit(&mut visitor as &mut dyn WindowVisitor);
    assert!(visitor.stack.is_empty());
    assert!(visitor.count > 30);
    let WidgetContents::Controls(old) = window.contents.contents else {
        panic!()
    };
    assert!(matches!(
        old.control(volume).unwrap().kind,
        ControlKind::Number { value: 100, .. }
    ));
}

#[test]
fn page_and_tab_activation_marks_selection() {
    let mut session = settings(true);
    let pages: Vec<(wakui::control::PageId, ControlId)> = session
        .screen()
        .pages
        .iter()
        .map(|page| (page.id, page.select.id))
        .collect();
    assert!(pages.len() > 1);
    for (page_id, select) in pages {
        send(&mut session, select, ControlInput::Activate);
        assert!(
            session
                .screen()
                .pages
                .iter()
                .find(|page| page.id == page_id)
                .unwrap()
                .selected
        );
    }
    let tabs: Vec<(wakui::control::TabId, ControlId)> = {
        struct Tabs(Vec<(wakui::control::TabId, ControlId)>);
        impl ControlVisitor for Tabs {
            fn begin_tab(&mut self, tab: &wakui::control::Tab) {
                self.0.push((tab.id, tab.select.id));
            }
        }
        let mut visitor = Tabs(Vec::new());
        session.screen().visit(&mut visitor);
        visitor.0
    };
    for (tab_id, select) in tabs {
        send(&mut session, select, ControlInput::Activate);
        struct Probe {
            target: wakui::control::TabId,
            selected: bool,
        }
        impl ControlVisitor for Probe {
            fn begin_tab(&mut self, tab: &wakui::control::Tab) {
                if tab.id == self.target {
                    self.selected = tab.selected;
                }
            }
        }
        let mut probe = Probe {
            target: tab_id,
            selected: false,
        };
        session.screen().visit(&mut probe);
        assert!(probe.selected);
    }
}
