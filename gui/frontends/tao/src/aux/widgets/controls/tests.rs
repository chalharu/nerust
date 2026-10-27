use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};

use crate::aux::window::WidgetBinding;
use iced::{
    Event, Point, Rectangle, Size,
    advanced::{
        Layout, Shell, Widget, layout, mouse, renderer,
        widget::{self, Tree},
    },
    keyboard::{self, key},
};
use nerust_core_traits::audio::AudioBackendRegistry;
use nerust_frontend_host::settings::{
    InputTab, SettingsControlId as Id, SettingsSystemId, SettingsTabId,
};
use nerust_gui_runtime::settings::SettingsSnapshot;
use nerust_gui_shell::registry::SystemRegistry;
use nerust_keyboard::Key;
use nerust_settings_core::model::{
    app_state::DesktopAppState,
    language::AppLanguage,
    local::HostBackendLocalSettings,
    shared::{DesktopSharedSettings, StoragePolicy},
};

use super::super::bindings::{SettingsAppState, SettingsBinding as SettingsAppProgram};
use super::*;

fn fixture(nes: bool) -> (SettingsAppProgram, SettingsAppState) {
    let mut shared = DesktopSharedSettings::default();
    shared.input.shortcuts.keyboard =
        nerust_settings_core::model::input::default_shortcut_bindings();
    let program = SettingsAppProgram::new(
        SettingsSnapshot {
            shared,
            local: HostBackendLocalSettings::default(),
            app_state: DesktopAppState::default(),
        },
        Arc::new(SystemRegistry::new(if nes {
            vec![nerust_nes_factory::NesFactory::boxed().into()]
        } else {
            vec![]
        })),
        Arc::new(AudioBackendRegistry::new()),
        Arc::new(AtomicBool::new(false)),
        Arc::new(Mutex::new(None)),
        Arc::new(AtomicBool::new(false)),
    );
    let (state, _) = program.boot();
    (program, state)
}

fn send(program: &SettingsAppProgram, state: &mut SettingsAppState, id: Id, input: ControlInput) {
    drop(program.update(state, Message::Input(id, input)));
}

#[test]
fn outer_window_consumes_the_actual_screen_and_program_title_uses_window_title() {
    use nerust_frontend_host::window::{WidgetContents, WindowVisitor};

    let (program, mut state) = fixture(false);
    state.error_message = Some("actual draft screen".into());
    state.page = nerust_frontend_host::settings::SettingsPage::Audio;
    let window = state.window();
    let WidgetContents::Settings(screen) = &window.contents.contents else {
        panic!("settings window must contain the actual screen");
    };
    assert_eq!(screen.message.as_deref(), Some("actual draft screen"));
    assert!(
        screen
            .pages
            .iter()
            .any(|page| page.selected && page.id == state.page)
    );
    let mut visitor = SettingsWindowVisitor::default();
    let outer: &mut dyn WindowVisitor = &mut visitor;
    window.visit(outer);
    let mut element = visitor
        .output
        .expect("outer visitor must invoke nested settings visitor");
    let (_, tree, _) = native_layout(&mut element);
    assert!(!tree.children.is_empty());
    // The title port reads the common window, not a standalone screen title.
    assert_eq!(program.presentation(&state).unwrap().title, window.title);
    // A native tree owns all text even after both projections are dropped.
    drop(window);
    drop(state);
    let (_, tree, _) = native_layout(&mut element);
    assert!(!tree.children.is_empty());
}

fn native_layout(element: &mut El<'static>) -> (iced_tiny_skia::Renderer, Tree, layout::Node) {
    let renderer =
        iced_tiny_skia::Renderer::new(crate::convert::default_font(), iced::Pixels(16.0));
    let mut tree = Tree::new(&*element);
    let node = element.as_widget_mut().layout(
        &mut tree,
        &renderer,
        &layout::Limits::new(Size::ZERO, Size::new(960.0, 720.0)),
    );
    (renderer, tree, node)
}

fn feed(
    element: &mut El<'static>,
    renderer: &iced_tiny_skia::Renderer,
    tree: &mut Tree,
    node: &layout::Node,
    events: &[Event],
    point: Point,
) -> Vec<Message> {
    let mut messages = vec![];
    let mut clipboard = iced::advanced::clipboard::Null;
    for event in events {
        element.as_widget_mut().update(
            tree,
            event,
            Layout::new(node),
            mouse::Cursor::Available(point),
            renderer,
            &mut clipboard,
            &mut Shell::new(&mut messages),
            &Rectangle::with_size(Size::new(960.0, 720.0)),
        );
    }
    messages
}

fn click() -> [Event; 2] {
    [
        Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)),
        Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)),
    ]
}

fn keys(key: keyboard::Key, code: key::Code, text: Option<&str>) -> [Event; 2] {
    [
        Event::Keyboard(keyboard::Event::KeyPressed {
            key: key.clone(),
            modified_key: key.clone(),
            physical_key: key::Physical::Code(code),
            location: keyboard::Location::Standard,
            modifiers: keyboard::Modifiers::empty(),
            text: text.map(Into::into),
            repeat: false,
        }),
        Event::Keyboard(keyboard::Event::KeyReleased {
            key: key.clone(),
            modified_key: key,
            physical_key: key::Physical::Code(code),
            location: keyboard::Location::Standard,
            modifiers: keyboard::Modifiers::empty(),
        }),
    ]
}

fn assert_ignored(
    element: &mut El<'static>,
    renderer: &iced_tiny_skia::Renderer,
    tree: &mut Tree,
    node: &layout::Node,
    events: &[Event],
    point: Point,
) {
    for event in events {
        let mut messages = vec![];
        let mut shell = Shell::new(&mut messages);
        element.as_widget_mut().update(
            tree,
            event,
            Layout::new(node),
            mouse::Cursor::Available(point),
            renderer,
            &mut iced::advanced::clipboard::Null,
            &mut shell,
            &Rectangle::with_size(Size::new(960.0, 720.0)),
        );
        assert_eq!(
            shell.event_status(),
            iced::event::Status::Ignored,
            "{event:?}"
        );
        assert!(messages.is_empty(), "{event:?}: {messages:?}");
    }
}

fn reconcile(
    element: &mut El<'static>,
    tree: &mut Tree,
    renderer: &iced_tiny_skia::Renderer,
) -> layout::Node {
    // Keep the existing runtime tree instead of constructing a fresh disabled tree.
    tree.diff(element.as_widget());
    element.as_widget_mut().layout(
        tree,
        renderer,
        &layout::Limits::new(Size::ZERO, Size::new(960.0, 720.0)),
    )
}

fn assert_no_overlay(
    element: &mut El<'static>,
    tree: &mut Tree,
    renderer: &iced_tiny_skia::Renderer,
    node: &layout::Node,
) {
    assert!(
        element
            .as_widget_mut()
            .overlay(
                tree,
                Layout::new(node),
                renderer,
                &Rectangle::with_size(Size::new(960.0, 720.0)),
                iced::Vector::ZERO,
            )
            .is_none()
    );
}

#[test]
fn disabled_native_controls_ignore_actual_key_presses_and_releases() {
    let (_, state) = fixture(false);
    let mut events = keys(
        keyboard::Key::Named(key::Named::ArrowUp),
        key::Code::ArrowUp,
        None,
    )
    .to_vec();
    // Positive control: these are real slider keyboard events, not just modifier updates.
    let mut control = state.screen().control(&Id::InfoDuration).unwrap().clone();
    control.metadata.enabled = true;
    control.kind = SettingsControlKind::Number {
        value: 10,
        min: 10,
        max: 30,
        step: 5,
        unit: "ticks".into(),
        formatted: "10 ticks".into(),
    };
    let mut element = control_element(&control, true, iced::Font::default());
    let (renderer, mut tree, node) = native_layout(&mut element);
    let point = Layout::new(&node)
        .children()
        .nth(1)
        .unwrap()
        .bounds()
        .center();
    let messages = feed(&mut element, &renderer, &mut tree, &node, &events, point);
    assert!(matches!(
        messages.as_slice(),
        [Message::Input(Id::InfoDuration, ControlInput::Number(15))]
    ));
    events.extend(keys(
        keyboard::Key::Character("v".into()),
        key::Code::KeyV,
        Some("v"),
    ));
    for id in [
        Id::Language,
        Id::InfoDuration,
        Id::Mute,
        Id::StorageDirectory,
        Id::Cancel,
    ] {
        let mut control = state.screen().control(&id).unwrap().clone();
        control.metadata.enabled = false;
        let mut element = control_element(&control, true, iced::Font::default());
        let (renderer, mut tree, node) = native_layout(&mut element);
        let point = match control.kind {
            SettingsControlKind::Choice { .. }
            | SettingsControlKind::Number { .. }
            | SettingsControlKind::Text { .. } => Layout::new(&node)
                .children()
                .nth(1)
                .unwrap()
                .bounds()
                .center(),
            _ => Layout::new(&node).bounds().center(),
        };
        assert_ignored(&mut element, &renderer, &mut tree, &node, &events, point);
    }
}

#[test]
fn disabling_an_open_native_pick_list_removes_overlay_and_ignores_input() {
    let (_, state) = fixture(false);
    // Exercise both the control's own flag and the enclosing layout's flag.
    for parent_enabled in [true, false] {
        let mut control = state.screen().control(&Id::Scaling).unwrap().clone();
        control.metadata.enabled = true;
        let mut element = control_element(&control, true, iced::Font::default());
        let (renderer, mut tree, node) = native_layout(&mut element);
        let point = Layout::new(&node)
            .children()
            .nth(1)
            .unwrap()
            .bounds()
            .center();
        assert!(feed(&mut element, &renderer, &mut tree, &node, &click(), point).is_empty());
        assert!(
            element
                .as_widget_mut()
                .overlay(
                    &mut tree,
                    Layout::new(&node),
                    &renderer,
                    &Rectangle::with_size(Size::new(960.0, 720.0)),
                    iced::Vector::ZERO,
                )
                .is_some(),
            "enabled pick list must actually be open"
        );

        control.metadata.enabled = !parent_enabled;
        element = control_element(&control, parent_enabled, iced::Font::default());
        let node = reconcile(&mut element, &mut tree, &renderer);
        assert_no_overlay(&mut element, &mut tree, &renderer, &node);
        let mut events = keys(
            keyboard::Key::Named(key::Named::ArrowUp),
            key::Code::ArrowUp,
            None,
        )
        .to_vec();
        events.push(Event::Keyboard(keyboard::Event::ModifiersChanged(
            keyboard::Modifiers::CTRL,
        )));
        events.push(Event::Mouse(mouse::Event::WheelScrolled {
            delta: mouse::ScrollDelta::Lines { x: 0.0, y: -1.0 },
        }));
        events.extend(click());
        events.extend([
            Event::Touch(iced::touch::Event::FingerPressed {
                id: iced::touch::Finger(0),
                position: point,
            }),
            Event::Touch(iced::touch::Event::FingerMoved {
                id: iced::touch::Finger(0),
                position: point,
            }),
            Event::Touch(iced::touch::Event::FingerLifted {
                id: iced::touch::Finger(0),
                position: point,
            }),
        ]);
        assert_ignored(&mut element, &renderer, &mut tree, &node, &events, point);
        assert_no_overlay(&mut element, &mut tree, &renderer, &node);
    }
}

#[test]
fn disabling_a_dragging_native_slider_ignores_motion_and_release() {
    let (_, state) = fixture(false);
    for parent_enabled in [true, false] {
        let mut control = state.screen().control(&Id::InfoDuration).unwrap().clone();
        control.metadata.enabled = true;
        control.kind = SettingsControlKind::Number {
            value: 10,
            min: 10,
            max: 30,
            step: 5,
            unit: "ticks".into(),
            formatted: "10 ticks".into(),
        };
        let mut element = control_element(&control, true, iced::Font::default());
        let (renderer, mut tree, node) = native_layout(&mut element);
        let bounds = Layout::new(&node).children().nth(1).unwrap().bounds();
        let point = bounds.center();
        let messages = feed(
            &mut element,
            &renderer,
            &mut tree,
            &node,
            &click()[..1],
            point,
        );
        assert!(matches!(
            messages.as_slice(),
            [Message::Input(Id::InfoDuration, ControlInput::Number(20))]
        ));
        let moved = Point::new(bounds.x + bounds.width, point.y);
        let motion = Event::Mouse(mouse::Event::CursorMoved { position: moved });
        let messages = feed(
            &mut element,
            &renderer,
            &mut tree,
            &node,
            std::slice::from_ref(&motion),
            moved,
        );
        assert!(
            matches!(
                messages.as_slice(),
                [Message::Input(Id::InfoDuration, ControlInput::Number(30))]
            ),
            "enabled slider must actually be dragging"
        );

        control.metadata.enabled = !parent_enabled;
        element = control_element(&control, parent_enabled, iced::Font::default());
        let node = reconcile(&mut element, &mut tree, &renderer);
        let mut events = vec![motion, click()[1].clone()];
        events.extend(keys(
            keyboard::Key::Named(key::Named::ArrowUp),
            key::Code::ArrowUp,
            None,
        ));
        events.push(Event::Touch(iced::touch::Event::FingerLifted {
            id: iced::touch::Finger(0),
            position: point,
        }));
        assert_ignored(&mut element, &renderer, &mut tree, &node, &events, point);
        assert_no_overlay(&mut element, &mut tree, &renderer, &node);
    }
}

#[derive(Default)]
struct EditableOperation {
    focusable: usize,
    editable: usize,
    custom: usize,
}

impl widget::Operation for EditableOperation {
    fn traverse(&mut self, operate: &mut dyn FnMut(&mut dyn widget::Operation)) {
        operate(self);
    }

    fn focusable(
        &mut self,
        _: Option<&widget::Id>,
        _: Rectangle,
        _: &mut dyn widget::operation::Focusable,
    ) {
        self.focusable += 1;
    }

    fn text_input(
        &mut self,
        _: Option<&widget::Id>,
        _: Rectangle,
        _: &mut dyn widget::operation::TextInput,
    ) {
        self.editable += 1;
    }

    fn custom(&mut self, _: Option<&widget::Id>, _: Rectangle, _: &mut dyn std::any::Any) {
        self.custom += 1;
    }
}

#[test]
fn disabled_gate_prevents_operating_real_editable_descendants() {
    let (_, state) = fixture(false);
    let mut control = state
        .screen()
        .control(&Id::StorageDirectory)
        .unwrap()
        .clone();
    control.metadata.enabled = true;
    for enabled in [true, false] {
        let mut element = super::super::disabled::gate(
            control_element(&control, true, iced::Font::default()),
            enabled,
        );
        let (renderer, mut tree, node) = native_layout(&mut element);
        let mut operation = EditableOperation::default();
        element
            .as_widget_mut()
            .operate(&mut tree, Layout::new(&node), &renderer, &mut operation);
        assert_eq!(operation.focusable, usize::from(enabled));
        assert_eq!(operation.editable, usize::from(enabled));
    }
}

struct OperationSentinel(Arc<std::sync::atomic::AtomicUsize>);

impl Widget<Message, iced::Theme, iced_tiny_skia::Renderer> for OperationSentinel {
    fn size(&self) -> Size<iced::Length> {
        Size::new(iced::Length::Shrink, iced::Length::Shrink)
    }

    fn layout(
        &mut self,
        _: &mut Tree,
        _: &iced_tiny_skia::Renderer,
        _: &layout::Limits,
    ) -> layout::Node {
        layout::Node::new(Size::new(40.0, 20.0))
    }

    fn draw(
        &self,
        _: &Tree,
        _: &mut iced_tiny_skia::Renderer,
        _: &iced::Theme,
        _: &renderer::Style,
        _: Layout<'_>,
        _: mouse::Cursor,
        _: &Rectangle,
    ) {
    }

    fn operate(
        &mut self,
        _: &mut Tree,
        layout: Layout<'_>,
        _: &iced_tiny_skia::Renderer,
        operation: &mut dyn widget::Operation,
    ) {
        self.0.fetch_add(1, Ordering::Relaxed);
        operation.custom(None, layout.bounds(), &mut ());
    }
}

#[test]
fn disabled_gate_never_calls_inner_operate() {
    let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    for enabled in [true, false] {
        calls.store(0, Ordering::Relaxed);
        let mut element = super::super::disabled::gate(
            iced::Element::new(OperationSentinel(calls.clone())),
            enabled,
        );
        let (renderer, mut tree, node) = native_layout(&mut element);
        let mut operation = EditableOperation::default();
        element
            .as_widget_mut()
            .operate(&mut tree, Layout::new(&node), &renderer, &mut operation);
        assert_eq!(calls.load(Ordering::Relaxed), usize::from(enabled));
        assert_eq!(operation.custom, usize::from(enabled));
    }
}

#[test]
fn disabling_a_focused_native_text_input_does_not_capture_key_release() {
    let (_, state) = fixture(false);
    let mut control = state
        .screen()
        .control(&Id::StorageDirectory)
        .unwrap()
        .clone();
    control.metadata.enabled = true;
    let mut element = control_element(&control, true, iced::Font::default());
    let (renderer, mut tree, node) = native_layout(&mut element);
    let point = Layout::new(&node)
        .children()
        .nth(1)
        .unwrap()
        .bounds()
        .center();
    feed(&mut element, &renderer, &mut tree, &node, &click(), point);
    let events = keys(
        keyboard::Key::Character("v".into()),
        key::Code::KeyV,
        Some("v"),
    );
    let messages = feed(&mut element, &renderer, &mut tree, &node, &events, point);
    assert!(
        matches!(messages.as_slice(), [Message::Input(Id::StorageDirectory, ControlInput::Text(value))] if value.contains('v'))
    );
    control.metadata.enabled = false;
    element = control_element(&control, true, iced::Font::default());
    let node = reconcile(&mut element, &mut tree, &renderer);
    assert_ignored(&mut element, &renderer, &mut tree, &node, &events, point);
}

#[test]
fn all_six_pages_build_native_trees_from_owned_empty_and_real_models() {
    for nes in [false, true] {
        let (_, mut state) = fixture(nes);
        state
            .vm
            .general
            .set_language(AppLanguage::Japanese)
            .unwrap();
        let pages = state.screen().pages;
        assert_eq!(pages.len(), 6);
        for page in pages {
            state.page = page.id;
            let revision = state.vm.revision.get();
            // Temporary screen is gone before iced even performs layout.
            let mut element: El<'static> = build(&state.window());
            let (_, tree, node) = native_layout(&mut element);
            assert!(!tree.children.is_empty());
            assert!(node.size().width > 0.0 && node.size().height > 0.0);
            assert_eq!(state.vm.revision.get(), revision);
        }
    }
}

#[derive(Default)]
struct Inventory(Vec<SettingsControl>, Vec<SettingsTab>);
impl SettingsVisitor for Inventory {
    fn control(&mut self, control: &SettingsControl) {
        self.0.push(control.clone());
    }
    fn begin_tab(&mut self, tab: &SettingsTab) {
        self.1.push(tab.clone());
    }
}

#[test]
fn every_control_variant_builds_enabled_and_disabled_without_borrowed_labels() {
    let (_, state) = fixture(true);
    let mut inventory = Inventory::default();
    state.screen().visit(&mut inventory);
    let mut kinds = [false; 7];
    for mut control in inventory.0 {
        kinds[match control.kind {
            SettingsControlKind::Toggle { .. } => 0,
            SettingsControlKind::Choice { .. } => 1,
            SettingsControlKind::Number { .. } => 2,
            SettingsControlKind::Text { .. } => 3,
            SettingsControlKind::Binding { .. } => 4,
            SettingsControlKind::Action => 5,
            SettingsControlKind::Note { .. } => 6,
        }] = true;
        for enabled in [true, false] {
            control.metadata.enabled = enabled;
            let mut element = control_element(&control, true, iced::Font::default());
            let (_, _, node) = native_layout(&mut element);
            assert!(node.size().height > 0.0);
        }
    }
    assert!(kinds.into_iter().all(|built| built));
}

#[test]
fn visitor_obeys_selected_hidden_disabled_layouts_and_balances_frames() {
    let (_, state) = fixture(true);
    let mut screen = state.screen();
    for page in &mut screen.pages {
        page.selected = false;
    }
    let page = &mut screen.pages[0];
    page.selected = true;
    let mut visitor = IcedSettingsVisitor::default();
    visitor.begin_screen(&screen);
    visitor.begin_page(&screen.pages[0]);
    let mut group = screen.pages[0].body.clone();
    group.metadata.visible = false;
    visitor.begin_group(&group);
    visitor.control(screen.control(&Id::Language).unwrap());
    assert!(visitor.stack.last().unwrap().children.is_empty());
    visitor.end_group(&group);
    assert!(visitor.stack.last().unwrap().children.is_empty());
    group.metadata.visible = true;
    group.metadata.enabled = false;
    visitor.begin_column(&group);
    assert!(!visitor.stack.last().unwrap().enabled);
    visitor.begin_row(&group);
    assert!(!visitor.stack.last().unwrap().enabled);
    visitor.control(screen.control(&Id::Language).unwrap());
    visitor.end_row(&group);
    visitor.end_column(&group);
    visitor.end_page(&screen.pages[0]);
    visitor.end_screen(&screen);
    assert!(visitor.stack.is_empty());
    assert!(visitor.output.is_some());
    // Traverse all nested groups/tabs through the object-safe host traversal.
    let mut visitor = IcedSettingsVisitor::default();
    screen.visit(&mut visitor as &mut dyn SettingsVisitor);
    assert!(visitor.stack.is_empty());
}

#[test]
fn native_navigation_emits_typed_activation_and_updates_all_pages_and_tabs() {
    let (program, mut state) = fixture(true);
    for page in state.screen().pages {
        let mut element = navigation(&page.select, false, true);
        let (renderer, mut tree, node) = native_layout(&mut element);
        let messages = feed(
            &mut element,
            &renderer,
            &mut tree,
            &node,
            &click(),
            Point::new(10.0, 10.0),
        );
        assert_eq!(messages.len(), 1);
        assert!(
            matches!(&messages[0], Message::Input(id, ControlInput::Activate) if id == &page.select.id)
        );
        for message in messages {
            drop(program.update(&mut state, message));
        }
        assert_eq!(state.page, page.id);
    }
    let mut inventory = Inventory::default();
    state.screen().visit(&mut inventory);
    for tab in inventory.1 {
        send(&program, &mut state, tab.select.id, ControlInput::Activate);
        let mut current = Inventory::default();
        state.screen().visit(&mut current);
        assert!(current.1.iter().find(|t| t.id == tab.id).unwrap().selected);
    }
}

#[test]
fn dynamic_ids_restore_context_before_editing_instead_of_using_selected_target() {
    let (program, mut state) = fixture(true);
    let view = state.vm.systems()[0].view.get();
    let field = &view.fields[0];
    let index = field
        .choices
        .iter()
        .position(|c| c.value != field.selected)
        .unwrap();
    let id = Id::SystemField {
        system: SettingsSystemId(view.system_id.to_string()),
        field: field.id.clone(),
    };
    state.system_tab_index = Some(999);
    send(&program, &mut state, id, ControlInput::Select(index));
    assert_eq!(state.system_tab_index, Some(0));
    assert_eq!(
        state.vm.systems()[0].view.get().fields[0].selected,
        field.choices[index].value
    );
    let view = state.vm.inputs()[0].view.get();
    let slot = &view.slots[0];
    let none = slot.choices.iter().position(|c| c.value.is_none()).unwrap();
    state.input_tab = Some(InputTab::Shortcuts);
    send(
        &program,
        &mut state,
        Id::ControllerSlot {
            system: SettingsSystemId(view.system_id.to_string()),
            slot: slot.slot_id,
        },
        ControlInput::Select(none),
    );
    assert_eq!(state.input_tab, Some(InputTab::System(0)));
    assert!(
        state.vm.inputs()[0].view.get().slots[0]
            .selected_profile_id
            .is_none()
    );
}

#[test]
fn stale_disabled_hidden_wrong_kind_and_missing_callbacks_are_ignored() {
    let (program, mut state) = fixture(false);
    state.vm.notifications.set_info_enabled(true).unwrap();
    let callback = Message::Input(Id::InfoDuration, ControlInput::Number(2000));
    state.vm.notifications.set_info_enabled(false).unwrap();
    let revision = state.vm.revision.get();
    state.error_message = Some("retain error on rejected input".into());
    for message in [
        callback,
        Message::Input(Id::Volume, ControlInput::Number(u32::MAX)),
        Message::Input(Id::Mute, ControlInput::Select(0)),
        Message::Input(Id::Language, ControlInput::Select(usize::MAX)),
        Message::Input(
            Id::Tab(SettingsTabId::System(SettingsSystemId("missing".into()))),
            ControlInput::Activate,
        ),
        Message::Input(Id::StorageDirectory, ControlInput::Text("hidden".into())),
    ] {
        drop(program.update(&mut state, message));
        assert_eq!(state.vm.revision.get(), revision);
        assert_eq!(
            state.error_message.as_deref(),
            Some("retain error on rejected input")
        );
    }
}

#[test]
fn capture_key_resolves_current_capture_and_clear_reaches_viewmodel() {
    let (program, mut state) = fixture(false);
    let target = state.vm.shortcuts.view.get().rows[0].target.clone();
    let id = Id::Binding(target.clone());
    drop(program.update(&mut state, Message::CaptureKey(Key::KeyZ)));
    assert!(state.screen().capture.target.is_none());
    send(&program, &mut state, id.clone(), ControlInput::Activate);
    assert_eq!(state.screen().capture.target, Some(target));
    drop(program.update(&mut state, Message::CaptureKey(Key::KeyZ)));
    assert!(state.screen().capture.target.is_none());
    assert!(matches!(
        state.screen().control(&id).unwrap().kind,
        SettingsControlKind::Binding {
            value: BindingValueView::Bound(_),
            ..
        }
    ));
    send(&program, &mut state, id.clone(), ControlInput::CaptureClear);
    assert!(matches!(
        state.screen().control(&id).unwrap().kind,
        SettingsControlKind::Binding {
            value: BindingValueView::Unbound(_),
            ..
        }
    ));
}

#[test]
fn text_validation_footer_submit_and_cancel_use_model_policy() {
    let (program, mut state) = fixture(false);
    state
        .vm
        .general
        .set_storage_policy(StoragePolicy::CustomDirectory)
        .unwrap();
    send(
        &program,
        &mut state,
        Id::StorageDirectory,
        ControlInput::Text(String::new()),
    );
    assert!(!state.screen().validation.can_submit);
    assert!(state.screen().message.is_some());
    send(&program, &mut state, Id::Submit, ControlInput::Activate);
    assert!(!program.should_close.load(Ordering::Acquire));
    assert!(program.pending_apply.lock().unwrap().is_none());
    assert!(matches!(
        state
            .screen()
            .bind(&Id::StorageDirectory, ControlInput::Activate)
            .unwrap()
            .last(),
        Some(nerust_frontend_host::settings::SettingsEvent::BrowseStorageDirectory)
    ));
    send(
        &program,
        &mut state,
        Id::StorageDirectory,
        ControlInput::Text("/tmp".into()),
    );
    assert!(state.screen().validation.can_submit);
    send(&program, &mut state, Id::Submit, ControlInput::Activate);
    assert!(program.should_close.load(Ordering::Acquire));
    assert!(program.pending_apply.lock().unwrap().is_some());
    let (program, mut state) = fixture(false);
    send(&program, &mut state, Id::Cancel, ControlInput::Activate);
    assert!(program.should_close.load(Ordering::Acquire));
    assert!(program.pending_apply.lock().unwrap().is_none());
}

#[test]
fn numeric_native_slider_honors_model_range_step_and_emits_ids() {
    let (_, state) = fixture(false);
    let mut control = state.screen().control(&Id::InfoDuration).unwrap().clone();
    control.metadata.enabled = true;
    control.kind = SettingsControlKind::Number {
        value: 10,
        min: 10,
        max: 30,
        step: 5,
        unit: "ticks".into(),
        formatted: "10 ticks".into(),
    };
    let mut element = control_element(&control, true, iced::Font::default());
    let (renderer, mut tree, node) = native_layout(&mut element);
    let slider = Layout::new(&node).children().nth(1).unwrap().bounds();
    let messages = feed(
        &mut element,
        &renderer,
        &mut tree,
        &node,
        &click(),
        slider.center(),
    );
    assert!(matches!(
        messages.as_slice(),
        [Message::Input(Id::InfoDuration, ControlInput::Number(20))]
    ));
}

#[test]
fn disabled_native_controls_never_publish_or_open_overlays() {
    let (_, state) = fixture(false);
    for id in [
        Id::Language,
        Id::InfoDuration,
        Id::Mute,
        Id::StorageDirectory,
        Id::Cancel,
    ] {
        let mut control = state.screen().control(&id).unwrap().clone();
        control.metadata.enabled = false;
        let mut element = control_element(&control, true, iced::Font::default());
        let (renderer, mut tree, node) = native_layout(&mut element);
        let point = match control.kind {
            SettingsControlKind::Choice { .. }
            | SettingsControlKind::Number { .. }
            | SettingsControlKind::Text { .. } => Layout::new(&node)
                .children()
                .nth(1)
                .unwrap()
                .bounds()
                .center(),
            _ => Layout::new(&node).bounds().center(),
        };
        let mut events = click().to_vec();
        events.push(Event::Keyboard(iced::keyboard::Event::ModifiersChanged(
            iced::keyboard::Modifiers::CTRL,
        )));
        events.push(Event::Mouse(mouse::Event::WheelScrolled {
            delta: mouse::ScrollDelta::Lines { x: 0.0, y: -1.0 },
        }));
        events.push(Event::Touch(iced::touch::Event::FingerPressed {
            id: iced::touch::Finger(0),
            position: point,
        }));
        assert!(feed(&mut element, &renderer, &mut tree, &node, &events, point).is_empty());
        assert!(
            element
                .as_widget_mut()
                .overlay(
                    &mut tree,
                    Layout::new(&node),
                    &renderer,
                    &Rectangle::with_size(Size::new(960.0, 720.0)),
                    iced::Vector::ZERO
                )
                .is_none()
        );
        assert!(matches!(
            element.as_widget().mouse_interaction(
                &tree,
                Layout::new(&node),
                mouse::Cursor::Available(point),
                &Rectangle::with_size(Size::new(960.0, 720.0)),
                &renderer
            ),
            mouse::Interaction::None | mouse::Interaction::Idle
        ));
    }
}

#[test]
fn native_choice_uses_indices_even_when_labels_are_identical() {
    let (_, state) = fixture(false);
    let mut control = state.screen().control(&Id::Scaling).unwrap().clone();
    control.kind = SettingsControlKind::Choice {
        choices: vec!["same".into(), "same".into()],
        selected: Some(0),
    };
    let mut element = control_element(&control, true, iced::Font::default());
    drop(control);
    let (renderer, mut tree, node) = native_layout(&mut element);
    let point = Layout::new(&node)
        .children()
        .nth(1)
        .unwrap()
        .bounds()
        .center();
    let messages = feed(
        &mut element,
        &renderer,
        &mut tree,
        &node,
        &[
            Event::Keyboard(iced::keyboard::Event::ModifiersChanged(
                iced::keyboard::Modifiers::CTRL,
            )),
            Event::Mouse(mouse::Event::WheelScrolled {
                delta: mouse::ScrollDelta::Lines { x: 0.0, y: -1.0 },
            }),
        ],
        point,
    );
    assert!(matches!(
        messages.as_slice(),
        [Message::Input(Id::Scaling, ControlInput::Select(1))]
    ));
}

#[test]
fn native_browse_change_clear_buttons_emit_only_current_control_ids() {
    let (_, state) = fixture(false);
    let screen = state.screen();
    let binding = state.vm.shortcuts.view.get().rows[0].target.clone();
    for (id, child, expected) in [
        (Id::StorageDirectory, 2, ControlInput::Activate),
        (Id::Binding(binding.clone()), 2, ControlInput::Activate),
        (Id::Binding(binding), 3, ControlInput::CaptureClear),
    ] {
        let mut control = screen.control(&id).unwrap().clone();
        control.metadata.enabled = true;
        let mut element = control_element(&control, true, iced::Font::default());
        let (renderer, mut tree, node) = native_layout(&mut element);
        let point = Layout::new(&node)
            .children()
            .nth(child)
            .unwrap()
            .bounds()
            .center();
        let messages = feed(&mut element, &renderer, &mut tree, &node, &click(), point);
        assert_eq!(
            format!("{messages:?}"),
            format!("{:?}", [Message::Input(id, expected)])
        );
    }
}

#[test]
fn topology_change_rejects_removed_binding_and_keyboard_uses_latest_capture() {
    let (program, mut state) = fixture(true);
    let input = state.vm.inputs()[0].view.get();
    let old = input
        .sections
        .iter()
        .flat_map(|s| &s.rows)
        .map(|r| r.target.clone())
        .collect::<Vec<_>>();
    for slot in &input.slots {
        state.vm.inputs()[0]
            .set_controller_slot(slot.slot_id, None)
            .unwrap();
    }
    let removed = old
        .into_iter()
        .find(|target| {
            state
                .screen()
                .control(&Id::Binding(target.clone()))
                .is_none()
        })
        .unwrap();
    let revision = state.vm.revision.get();
    send(
        &program,
        &mut state,
        Id::Binding(removed),
        ControlInput::Activate,
    );
    assert_eq!(state.vm.revision.get(), revision);
    assert!(state.screen().capture.target.is_none());
    let shortcuts = state.vm.shortcuts.view.get();
    let first = Id::Binding(shortcuts.rows[0].target.clone());
    let second = Id::Binding(shortcuts.rows[1].target.clone());
    send(&program, &mut state, first, ControlInput::Activate);
    let callback = Message::CaptureKey(Key::KeyZ);
    send(&program, &mut state, second.clone(), ControlInput::Activate);
    drop(program.update(&mut state, callback));
    assert!(state.screen().capture.target.is_none());
    assert!(matches!(&state.screen().control(&second).unwrap().kind,
        SettingsControlKind::Binding { value: BindingValueView::Bound(label), .. } if label.contains('Z')));
}
