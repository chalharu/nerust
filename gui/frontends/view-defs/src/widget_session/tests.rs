use super::*;
use wakui::{
    scene::control::{
        Container, Control, ControlInput, ControlVisitor, IdRegistry, Page, Tab, Tabs,
    },
    scene::leaf::{BindingLeaf, ChoiceLeaf, NoteLeaf, NumberLeaf, TextLeaf},
};

use crate::settings::{SettingsControlId, SettingsSystemId, screen::StableKey};
use nerust_core_traits::audio::AudioBackendRegistry;
use nerust_gui_shell::registry::SystemRegistry;
use nerust_settings_core::{
    editor::CaptureTarget,
    model::{input::ShortcutAction, shared::StoragePolicy},
};
use wakui::chrome::window::WidgetContents;
use wakui::chrome::window::WindowVisitor;

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
    controller.ids.borrow_mut().id(StableKey::Control(domain))
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
    assert_eq!(
        session
            .screen()
            .control(volume)
            .unwrap()
            .downcast::<NumberLeaf>()
            .unwrap()
            .value,
        37
    );

    let mut history = WidgetSession::history(Arc::new([]), AppLanguage::English);
    let filter = history.history_ids().filter;
    send(&mut history, filter, ControlInput::Select(3));
    assert_eq!(
        history
            .screen()
            .control(filter)
            .unwrap()
            .downcast::<ChoiceLeaf>()
            .unwrap()
            .selected,
        Some(3)
    );
    let clear = history.history_ids().clear;
    send(&mut history, clear, ControlInput::Activate);
    assert!(history.mailbox().has_pending());
    assert!(matches!(
        take_submission(&history.mailbox()).unwrap().take(),
        Some(Pending::ClearHistory)
    ));
    let close = history.history_ids().close;
    send(&mut history, close, ControlInput::Activate);
    assert!(history.mailbox().close_requested());
    for session in [session, history] {
        let WidgetContents::Controls(screen) = session
            .window(wakui::chrome::aux::AuxId::from_raw(7))
            .contents
            .contents
        else {
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
    let filter = session.history_ids().filter;
    let rows = |session: &WidgetSession| {
        let mut out = Vec::new();
        for control in inventory(&session.screen()) {
            if let Some(list) = control.downcast::<wakui::scene::leaf::ListLeaf>() {
                for row in &list.rows {
                    for cell in &row.cells {
                        if let Some(note) = cell.downcast::<NoteLeaf>()
                            && note.text.starts_with("row-")
                        {
                            out.push(note.text.clone());
                        }
                    }
                }
            }
        }
        out
    };
    assert_eq!(rows(&session).len(), 3);
    send(&mut session, filter, ControlInput::Select(2));
    assert_eq!(rows(&session), ["row-Warning", "row-Error"]);
    send(&mut session, filter, ControlInput::Select(usize::MAX));
    assert_eq!(rows(&session).len(), 2);
    send(&mut session, filter, ControlInput::Select(3));
    assert_eq!(rows(&session), ["row-Error"]);
    assert_eq!(session.history_ids().filter, filter);
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
    assert_eq!(
        session
            .screen()
            .control(volume)
            .unwrap()
            .downcast::<NumberLeaf>()
            .unwrap()
            .value,
        29
    );
}

#[test]
fn minted_ids_survive_unrelated_rebuilds_and_never_alias() {
    let mut session = settings(true);
    let before: Vec<(String, ControlId)> = inventory(&session.screen())
        .into_iter()
        .map(|c| (c.label.clone(), c.id))
        .collect();
    assert!(!before.is_empty());
    let volume = token(&session, SettingsControlId::Volume);
    send(&mut session, volume, ControlInput::Number(31));
    let after: Vec<(String, ControlId)> = inventory(&session.screen())
        .into_iter()
        .map(|c| (c.label.clone(), c.id))
        .collect();
    assert_eq!(before, after, "unrelated edits must not reissue IDs");
    let history = WidgetSession::history(Arc::new([]), AppLanguage::English);
    let history_ids: Vec<ControlId> = inventory(&history.screen())
        .into_iter()
        .map(|c| c.id)
        .collect();
    for (_, id) in &before {
        assert!(
            !history_ids.contains(id),
            "fresh registries must not alias another owner"
        );
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
    assert_eq!(
        session
            .screen()
            .control(volume)
            .unwrap()
            .downcast::<NumberLeaf>()
            .unwrap()
            .value,
        100
    );
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
    assert_eq!(
        session
            .screen()
            .control(directory)
            .unwrap()
            .downcast::<TextLeaf>()
            .unwrap()
            .value,
        "/tmp"
    );
    session.submit(directory, *request, Some("/stale".into()));
    assert_eq!(
        session
            .screen()
            .control(directory)
            .unwrap()
            .downcast::<TextLeaf>()
            .unwrap()
            .value,
        "/tmp"
    );
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
    let screen = session.screen();
    let bound = screen.control(binding).unwrap();
    assert!(!bound.downcast::<BindingLeaf>().unwrap().capturing);
    assert!(
        bound
            .downcast::<BindingLeaf>()
            .unwrap()
            .text
            .contains("Space")
    );
}

#[test]
fn owned_generic_visitors_reenter_with_no_controller_borrow() {
    struct Reenter<'a> {
        session: &'a RefCell<WidgetSession>,
        volume: ControlId,
        stack: Vec<wakui::scene::control::Id>,
        count: usize,
    }
    impl ControlVisitor for Reenter<'_> {
        fn begin_page(&mut self, page: &Page) {
            self.stack.push(page.id);
        }
        fn end_page(&mut self, page: &Page) {
            assert_eq!(self.stack.pop(), Some(page.id));
        }
        fn begin_container(&mut self, container: &Container) {
            self.stack.push(container.id);
        }
        fn end_container(&mut self, container: &Container) {
            assert_eq!(self.stack.pop(), Some(container.id));
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
    let window = session
        .borrow()
        .window(wakui::chrome::aux::AuxId::from_raw(7));
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
    assert_eq!(
        old.control(volume)
            .unwrap()
            .downcast::<NumberLeaf>()
            .unwrap()
            .value,
        100
    );
}

#[test]
fn page_and_tab_activation_marks_selection() {
    let mut session = settings(true);
    let pages: Vec<(wakui::scene::control::PageId, ControlId)> = session
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
    let tabs: Vec<(wakui::scene::control::TabId, ControlId)> = {
        struct Tabs(Vec<(wakui::scene::control::TabId, ControlId)>);
        impl ControlVisitor for Tabs {
            fn begin_tab(&mut self, tab: &wakui::scene::control::Tab) {
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
            target: wakui::scene::control::TabId,
            selected: bool,
        }
        impl ControlVisitor for Probe {
            fn begin_tab(&mut self, tab: &wakui::scene::control::Tab) {
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

fn find_by_label(screen: &ControlScreen, label: &str) -> Control {
    inventory(screen)
        .into_iter()
        .find(|c| c.label == label)
        .unwrap_or_else(|| panic!("control {label:?} not found"))
}

fn controls_in(container: &Container) -> Vec<Control> {
    use wakui::scene::control::Node;
    container
        .children
        .iter()
        .flat_map(|node| match node {
            Node::Control(c) => vec![c.clone()],
            Node::Container(inner) => controls_in(inner),
            Node::Tabs(t) => t.tabs.iter().flat_map(|t| controls_in(&t.body)).collect(),
        })
        .collect()
}

#[derive(Default)]
struct TabInventory(Vec<Tab>);
impl ControlVisitor for TabInventory {
    fn begin_tab(&mut self, tab: &Tab) {
        self.0.push(tab.clone());
    }
}

#[test]
fn capture_key_resolves_current_capture_and_clear_reaches_model() {
    use wakui::scene::control::ControlEvent;
    let mut session = settings(false);
    let id = inventory(&session.screen())
        .into_iter()
        .find(|c| c.downcast::<BindingLeaf>().is_some())
        .unwrap()
        .id;
    session.dispatch(ControlEvent::capture_key(wakui::Key::KeyZ));
    assert!(session.screen().capture.target.is_none());
    send(&mut session, id, ControlInput::Activate);
    assert_eq!(session.screen().capture.target, Some(id));
    session.dispatch(ControlEvent::capture_key(wakui::Key::KeyZ));
    assert!(session.screen().capture.target.is_none());
    let screen = session.screen();
    let bound = screen.control(id).unwrap();
    assert!(!bound.downcast::<BindingLeaf>().unwrap().capturing);
    assert!(bound.downcast::<BindingLeaf>().unwrap().text.contains('Z'));
    send(&mut session, id, ControlInput::CaptureClear);
    let cleared_screen = session.screen();
    let cleared = cleared_screen.control(id).unwrap();
    assert!(!cleared.downcast::<BindingLeaf>().unwrap().capturing);
    assert!(
        !cleared
            .downcast::<BindingLeaf>()
            .unwrap()
            .text
            .contains('Z')
    );
}

#[test]
fn text_validation_footer_submit_and_cancel_follow_model_policy() {
    use wakui::chrome::window::Window;
    let mut session = settings(false);
    let policy = find_by_label(&session.screen(), "Save storage policy");
    let choices = &policy.downcast::<ChoiceLeaf>().unwrap().choices;
    let custom = choices
        .iter()
        .position(|c| c.to_lowercase().contains("custom"))
        .unwrap();
    send(&mut session, policy.id, ControlInput::Select(custom));
    let directory = find_by_label(&session.screen(), "Save storage directory").id;
    let submit = find_by_label(&session.screen(), "OK").id;
    send(&mut session, directory, ControlInput::Text(String::new()));
    assert!(!session.screen().control(submit).unwrap().metadata.enabled);
    assert!(session.screen().message.is_some());
    send(&mut session, submit, ControlInput::Activate);
    assert!(!session.mailbox().close_requested());
    assert!(!session.mailbox().has_pending());
    let effects = session.dispatch(ControlEvent::new(directory, ControlInput::Activate));
    assert!(
        matches!(effects.as_slice(), [wakui::scene::control::ControlEffect::PickDirectory { target, .. }] if *target == directory)
    );
    send(&mut session, directory, ControlInput::Text("/tmp".into()));
    assert!(session.screen().control(submit).unwrap().metadata.enabled);
    send(&mut session, submit, ControlInput::Activate);
    assert!(session.mailbox().close_requested());
    assert!(session.mailbox().has_pending());
    let mut session = settings(false);
    let cancel = find_by_label(&session.screen(), "Cancel").id;
    send(&mut session, cancel, ControlInput::Activate);
    assert!(session.mailbox().close_requested());
    assert!(!session.mailbox().has_pending());
    let _ = Window::main("test");
}

#[test]
fn topology_change_rejects_removed_binding_and_uses_latest_capture() {
    use wakui::scene::control::ControlEvent;
    let mut session = settings(true);
    let mut tabs = TabInventory::default();
    session.screen().visit(&mut tabs);
    let input = tabs
        .0
        .iter()
        .find(|t| {
            controls_in(&t.body).iter().any(|c| {
                c.downcast::<ChoiceLeaf>()
                    .is_some_and(|leaf| leaf.choices.iter().any(|s| s.contains("None")))
            })
        })
        .unwrap()
        .clone();
    let old: Vec<_> = controls_in(&input.body)
        .iter()
        .filter(|c| c.downcast::<BindingLeaf>().is_some())
        .map(|c| c.id)
        .collect();
    for slot in controls_in(&input.body) {
        if let Some(leaf) = slot.downcast::<ChoiceLeaf>()
            && let Some(none) = leaf.choices.iter().position(|c| c.contains("None"))
        {
            send(&mut session, slot.id, ControlInput::Select(none));
        }
    }
    let removed = old
        .into_iter()
        .find(|id| session.screen().control(*id).is_none())
        .unwrap();
    let before = session.screen();
    send(&mut session, removed, ControlInput::Activate);
    assert_eq!(session.screen(), before);
    assert!(session.screen().capture.target.is_none());
    let bindings: Vec<_> = inventory(&session.screen())
        .into_iter()
        .filter(|c| c.downcast::<BindingLeaf>().is_some())
        .collect();
    let first = bindings[0].id;
    let second = bindings[1].id;
    send(&mut session, first, ControlInput::Activate);
    send(&mut session, second, ControlInput::Activate);
    session.dispatch(ControlEvent::capture_key(wakui::Key::KeyZ));
    assert!(session.screen().capture.target.is_none());
    let screen = session.screen();
    let second_leaf = screen.control(second).unwrap();
    assert!(!second_leaf.downcast::<BindingLeaf>().unwrap().capturing);
    assert!(
        second_leaf
            .downcast::<BindingLeaf>()
            .unwrap()
            .text
            .contains('Z')
    );
}

#[test]
fn dynamic_ids_restore_context_before_editing_instead_of_selected_target() {
    let mut session = settings(true);
    let screen = session.screen();
    let mut tabs = TabInventory::default();
    screen.visit(&mut tabs);
    let system = tabs
        .0
        .iter()
        .find(|t| t.label.contains("NES"))
        .unwrap()
        .clone();
    let candidate = controls_in(&system.body)
        .into_iter()
        .find(|c| {
            c.downcast::<ChoiceLeaf>()
                .is_some_and(|leaf| leaf.choices.len() > 1)
        })
        .unwrap();
    let selected = candidate.downcast::<ChoiceLeaf>().unwrap().selected;
    let index = usize::from(selected == Some(0));
    send(
        &mut session,
        screen.pages[0].select.id,
        ControlInput::Activate,
    );
    send(&mut session, candidate.id, ControlInput::Select(index));
    assert_eq!(
        session
            .screen()
            .control(candidate.id)
            .unwrap()
            .downcast::<ChoiceLeaf>()
            .unwrap()
            .selected,
        Some(index)
    );
    let mut current = TabInventory::default();
    session.screen().visit(&mut current);
    assert!(
        current
            .0
            .iter()
            .find(|tab| tab.id == system.id)
            .unwrap()
            .selected
    );
    let input = current
        .0
        .iter()
        .find(|tab| {
            controls_in(&tab.body).iter().any(|c| {
                c.downcast::<ChoiceLeaf>()
                    .is_some_and(|leaf| leaf.choices.iter().any(|choice| choice.contains("None")))
            })
        })
        .unwrap()
        .clone();
    let slot = controls_in(&input.body)
        .into_iter()
        .find(|c| {
            c.downcast::<ChoiceLeaf>()
                .is_some_and(|leaf| leaf.choices.iter().any(|choice| choice.contains("None")))
        })
        .unwrap();
    let choices = &slot.downcast::<ChoiceLeaf>().unwrap().choices;
    let none = choices
        .iter()
        .position(|choice| choice.contains("None"))
        .unwrap();
    let shortcuts = current
        .0
        .iter()
        .find(|tab| tab.label.contains("Shortcut"))
        .unwrap();
    send(&mut session, shortcuts.select.id, ControlInput::Activate);
    send(&mut session, slot.id, ControlInput::Select(none));
    assert_eq!(
        session
            .screen()
            .control(slot.id)
            .unwrap()
            .downcast::<ChoiceLeaf>()
            .unwrap()
            .selected,
        Some(none)
    );
    let mut current = TabInventory::default();
    session.screen().visit(&mut current);
    assert!(
        current
            .0
            .iter()
            .find(|tab| tab.id == input.id)
            .unwrap()
            .selected
    );
}
