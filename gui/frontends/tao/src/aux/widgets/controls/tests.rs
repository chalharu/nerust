use std::sync::{Arc, atomic::Ordering};

use iced::{
    Event, Point, Rectangle, Size,
    advanced::{
        Layout, Shell, Widget, layout, mouse, renderer,
        widget::{self, Tree},
    },
    keyboard::{self, key},
};
use nerust_frontend_host::widget_session::WidgetSession;
use nerust_settings_core::{
    i18n::{UiText, text as caption},
    model::language::AppLanguage,
};
use wakui::Key;
use wakui::control::{Control, ControlEvent, ControlId, ControlKind, ControlVisitor, Tab};

use super::super::bindings::tests::{controls, find, prepare};
use super::*;

fn fixture(nes: bool) -> WidgetSession {
    let mut host = if nes {
        crate::runtime::host::TestHost::with_nes()
    } else {
        crate::runtime::host::TestHost::new()
    }
    .host;
    prepare(&mut host, crate::aux::focus::AuxKind::Settings).1
}

fn send(state: &mut WidgetSession, id: ControlId, input: ControlInput) {
    state.dispatch(Message::new(id, input));
}

fn field(state: &WidgetSession, key: UiText) -> Control {
    find(&state.screen(), caption(AppLanguage::English, key))
}

#[test]
fn outer_window_consumes_the_actual_screen_and_program_title_uses_window_title() {
    use nerust_frontend_host::window::{WidgetContents, WindowVisitor};

    let mut state = fixture(false);
    let page = state.screen().pages[3].clone();
    send(&mut state, page.select.id, ControlInput::Activate);
    let window = state.window();
    let WidgetContents::Controls(screen) = &window.contents.contents else {
        panic!("settings window must contain the actual screen");
    };
    assert!(
        screen
            .pages
            .iter()
            .any(|current| current.selected && current.id == page.id)
    );
    let mut visitor = ControlsWindowVisitor::default();
    let outer: &mut dyn WindowVisitor = &mut visitor;
    window.visit(outer);
    let mut element = visitor
        .output
        .expect("outer visitor must invoke nested settings visitor");
    let (_, tree, _) = native_layout(&mut element);
    assert!(!tree.children.is_empty());
    // The title port reads the common window, not a standalone screen title.
    assert_eq!(state.screen().title, window.title);
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

// Domain-free scenes: no history/settings presenter or content-specific widths.
fn table_layout(children: Vec<wakui::control::Node>) -> super::Layout {
    let mut ids = wakui::control::IdRegistry::default();
    super::Layout {
        id: ids.id(()),
        label: String::new(),
        metadata: ControlMetadata::new("layout"),
        spacing: 8,
        indent: 0,
        header: None,
        children,
    }
}

fn table_note(value: &str, heading: bool) -> Control {
    let mut ids = wakui::control::IdRegistry::default();
    Control {
        id: ids.id(()),
        label: String::new(),
        metadata: ControlMetadata::new(value),
        display: if heading {
            ControlDisplay::Heading
        } else {
            ControlDisplay::Note
        },
        kind: ControlKind::Note { text: value.into() },
    }
}

fn table_row(values: &[&str], heading: bool) -> wakui::control::Node {
    use wakui::control::Node;
    Node::Row(table_layout(
        values
            .iter()
            .map(|value| Node::Control(table_note(value, heading)))
            .collect(),
    ))
}

fn table_screen(body: super::Layout) -> ControlScreen {
    let mut ids = wakui::control::IdRegistry::default();
    ControlScreen {
        title: "Generic controls".into(),
        pages: vec![Page {
            id: ids.id("page"),
            label: "Page".into(),
            selected: true,
            select: Control {
                kind: ControlKind::Action,
                ..table_note("Page", false)
            },
            body,
        }],
        footer: table_layout(vec![]),
        message: None,
        capture: wakui::control::Capture {
            target: None,
            prompt: String::new(),
        },
    }
}

fn horizontal_rows<'a>(node: Layout<'a>, columns: usize, rows: &mut Vec<Layout<'a>>) {
    let children: Vec<_> = node.children().collect();
    if children.len() == columns
        && children
            .windows(2)
            .all(|pair| pair[0].bounds().x + pair[0].bounds().width <= pair[1].bounds().x + 0.01)
    {
        rows.push(node);
    }
    for child in children {
        horizontal_rows(child, columns, rows);
    }
}

#[test]
fn generic_table_shares_real_column_offsets_and_wraps_variable_content() {
    use wakui::control::Node;
    let long = "a long message with variable content ".repeat(80);
    let mut table = table_layout(vec![
        table_row(&["Time", "Severity", "Source", "Message"], true),
        table_row(&["12:34:56.789", "I", "subsystem", "short"], false),
        table_row(&["1", "Warning", "X", &long], false),
        table_row(&["ragged", "row"], false),
    ]);
    table.label = "Table title".into();
    table.header = Some(table_note("Table description", false));
    table.indent = 11;
    let screen = table_screen(table_layout(vec![Node::Table(table)]));
    for width in [960.0, 640.0] {
        let mut element = build_screen(&screen);
        let renderer =
            iced_tiny_skia::Renderer::new(crate::convert::default_font(), iced::Pixels(16.0));
        let mut tree = Tree::new(&element);
        let node = element.as_widget_mut().layout(
            &mut tree,
            &renderer,
            &layout::Limits::new(Size::ZERO, Size::new(width, 720.0)),
        );
        let mut rows = vec![];
        horizontal_rows(Layout::new(&node), 4, &mut rows);
        assert_eq!(rows.len(), 4, "header, data and ragged rows must survive");
        let reference: Vec<_> = rows[0].children().map(|cell| cell.bounds()).collect();
        for row in &rows {
            assert!(row.bounds().x + row.bounds().width <= width);
            for (index, cell) in row.children().enumerate() {
                let bounds = cell.bounds();
                assert!((bounds.x - reference[index].x).abs() < 0.01);
                assert!((bounds.width - reference[index].width).abs() < 0.01);
                assert!((bounds.width - reference[0].width).abs() < 0.01);
                for content in cell.children() {
                    assert!(content.bounds().width <= bounds.width + 0.01);
                }
            }
        }
        let header_message = rows[0].children().nth(3).unwrap();
        let long_message = rows[2].children().nth(3).unwrap();
        assert!(long_message.bounds().height > header_message.bounds().height * 2.0);
        assert!(
            (rows[1].bounds().y - rows[0].bounds().y - rows[0].bounds().height - 8.0).abs() < 0.01
        );
    }
}

#[test]
fn generic_table_preserves_nested_layouts_headers_visibility_and_disabled_input() {
    use wakui::control::Node;
    let mut action = table_note("Action", false);
    action.label = "Action".into();
    action.kind = ControlKind::Action;
    let mut inner = table_layout(vec![table_row(&["nested", "table", "cells"], false)]);
    inner.header = Some(table_note("Nested header", true));
    let mut hidden = table_layout(vec![table_row(&["hidden"; 4], false)]);
    hidden.metadata.visible = false;
    let mut disabled_row = table_layout(vec![
        Node::Control(action.clone()),
        Node::Column(table_layout(vec![Node::Table(inner)])),
        Node::Control(table_note("third", false)),
        Node::Control(table_note("fourth", false)),
    ]);
    disabled_row.metadata.enabled = false;
    disabled_row.indent = 9;
    disabled_row.spacing = 13;
    let mut table = table_layout(vec![
        table_row(&["one", "two", "three", "four"], true),
        Node::Row(disabled_row),
        Node::Table(hidden),
    ]);
    table.header = Some(table_note("Visible table header", true));
    table.spacing = 17;
    table.indent = 5;
    let mut element = build_screen(&table_screen(table_layout(vec![Node::Table(
        table.clone(),
    )])));
    let (renderer, mut tree, node) = native_layout(&mut element);
    let mut rows = vec![];
    horizontal_rows(Layout::new(&node), 4, &mut rows);
    assert_eq!(rows.len(), 2);
    let first: Vec<_> = rows[0].children().collect();
    let second: Vec<_> = rows[1].children().collect();
    for (a, b) in first.iter().zip(&second) {
        assert!((a.bounds().x - b.bounds().x).abs() < 0.01);
    }
    assert!((first[0].bounds().x - rows[0].bounds().x - 9.0).abs() < 0.01);
    assert!(
        (first[1].bounds().x - first[0].bounds().x - first[0].bounds().width - 13.0).abs() < 0.01
    );
    assert!(
        (rows[1].bounds().y - rows[0].bounds().y - rows[0].bounds().height - 17.0).abs() < 0.01
    );
    let mut nested = vec![];
    horizontal_rows(Layout::new(&node), 3, &mut nested);
    assert_eq!(nested.len(), 1, "nested table keeps its independent tracks");
    let point = second[0].children().next().unwrap().bounds().center();
    assert!(feed(&mut element, &renderer, &mut tree, &node, &click(), point).is_empty());

    // Positive control proves the pointer really targets the native button.
    let Node::Row(row) = &mut table.children[1] else {
        unreachable!()
    };
    row.metadata.enabled = true;
    let mut element = build_screen(&table_screen(table_layout(vec![Node::Table(
        table.clone(),
    )])));
    let (renderer, mut tree, node) = native_layout(&mut element);
    assert_eq!(
        feed(&mut element, &renderer, &mut tree, &node, &click(), point),
        vec![Message::new(action.id, ControlInput::Activate)]
    );

    table.metadata.visible = false;
    let mut element = build_screen(&table_screen(table_layout(vec![Node::Table(table)])));
    let (_, _, node) = native_layout(&mut element);
    let mut rows = vec![];
    horizontal_rows(Layout::new(&node), 4, &mut rows);
    assert!(rows.is_empty());
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
    let state = fixture(false);
    let mut events = keys(
        keyboard::Key::Named(key::Named::ArrowUp),
        key::Code::ArrowUp,
        None,
    )
    .to_vec();
    // Positive control: these are real slider keyboard events, not just modifier updates.
    let mut control = field(&state, UiText::NotificationDuration);
    control.metadata.enabled = true;
    control.kind = ControlKind::Number {
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
        [ControlEvent {
            input: ControlInput::Number(15),
            ..
        }]
    ));
    events.extend(keys(
        keyboard::Key::Character("v".into()),
        key::Code::KeyV,
        Some("v"),
    ));
    for key in [
        UiText::Language,
        UiText::NotificationDuration,
        UiText::Mute,
        UiText::SaveStorageDirectory,
        UiText::Cancel,
    ] {
        let mut control = field(&state, key);
        control.metadata.enabled = false;
        let mut element = control_element(&control, true, iced::Font::default());
        let (renderer, mut tree, node) = native_layout(&mut element);
        let point = match control.kind {
            ControlKind::Choice { .. } | ControlKind::Number { .. } | ControlKind::Text { .. } => {
                Layout::new(&node)
                    .children()
                    .nth(1)
                    .unwrap()
                    .bounds()
                    .center()
            }
            _ => Layout::new(&node).bounds().center(),
        };
        assert_ignored(&mut element, &renderer, &mut tree, &node, &events, point);
    }
}

#[test]
fn disabling_an_open_native_pick_list_removes_overlay_and_ignores_input() {
    let state = fixture(false);
    // Exercise both the control's own flag and the enclosing layout's flag.
    for parent_enabled in [true, false] {
        let mut control = field(&state, UiText::Scaling);
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
    let state = fixture(false);
    for parent_enabled in [true, false] {
        let mut control = field(&state, UiText::NotificationDuration);
        control.metadata.enabled = true;
        control.kind = ControlKind::Number {
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
            [ControlEvent {
                input: ControlInput::Number(20),
                ..
            }]
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
                [ControlEvent {
                    input: ControlInput::Number(30),
                    ..
                }]
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
    let state = fixture(false);
    let mut control = field(&state, UiText::SaveStorageDirectory);
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
    let state = fixture(false);
    let mut control = field(&state, UiText::SaveStorageDirectory);
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
        matches!(messages.as_slice(), [ControlEvent { input: ControlInput::Text(value), .. }] if value.contains('v'))
    );
    control.metadata.enabled = false;
    element = control_element(&control, true, iced::Font::default());
    let node = reconcile(&mut element, &mut tree, &renderer);
    assert_ignored(&mut element, &renderer, &mut tree, &node, &events, point);
}

#[test]
fn all_six_pages_build_native_trees_from_owned_empty_and_real_models() {
    for nes in [false, true] {
        let mut state = fixture(nes);
        let language = field(&state, UiText::Language);
        send(&mut state, language.id, ControlInput::Select(2));
        let pages = state.screen().pages;
        assert_eq!(pages.len(), 6);
        for page in pages {
            send(&mut state, page.select.id, ControlInput::Activate);
            let before = state.screen();
            // Temporary screen is gone before iced even performs layout.
            let mut element: El<'static> = build(&state.window());
            let (_, tree, node) = native_layout(&mut element);
            assert!(!tree.children.is_empty());
            assert!(node.size().width > 0.0 && node.size().height > 0.0);
            assert_eq!(state.screen(), before);
        }
    }
}

#[derive(Default)]
struct Inventory(Vec<Control>, Vec<Tab>);
impl ControlVisitor for Inventory {
    fn control(&mut self, control: &Control) {
        self.0.push(control.clone());
    }
    fn begin_tab(&mut self, tab: &Tab) {
        self.1.push(tab.clone());
    }
}

#[test]
fn every_control_variant_builds_enabled_and_disabled_without_borrowed_labels() {
    let state = fixture(true);
    let mut inventory = Inventory::default();
    state.screen().visit(&mut inventory);
    let mut kinds = [false; 7];
    for mut control in inventory.0 {
        kinds[match control.kind {
            ControlKind::Toggle { .. } => 0,
            ControlKind::Choice { .. } => 1,
            ControlKind::Number { .. } => 2,
            ControlKind::Text { .. } => 3,
            ControlKind::Binding { .. } => 4,
            ControlKind::Action => 5,
            ControlKind::Note { .. } => 6,
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
    let state = fixture(true);
    let mut screen = state.screen();
    for page in &mut screen.pages {
        page.selected = false;
    }
    let page = &mut screen.pages[0];
    page.selected = true;
    let mut visitor = IcedControlVisitor::default();
    visitor.begin_screen(&screen);
    visitor.begin_page(&screen.pages[0]);
    let mut group = screen.pages[0].body.clone();
    group.metadata.visible = false;
    visitor.begin_layout(&group, LayoutKind::Group);
    visitor.control(&field(&state, UiText::Language));
    assert!(visitor.stack.last().unwrap().children.is_empty());
    visitor.end_layout(&group, LayoutKind::Group);
    assert!(visitor.stack.last().unwrap().children.is_empty());
    group.metadata.visible = true;
    group.metadata.enabled = false;
    visitor.begin_layout(&group, LayoutKind::Column);
    assert!(!visitor.stack.last().unwrap().enabled);
    visitor.begin_layout(&group, LayoutKind::Row);
    assert!(!visitor.stack.last().unwrap().enabled);
    visitor.control(&field(&state, UiText::Language));
    visitor.end_layout(&group, LayoutKind::Row);
    visitor.end_layout(&group, LayoutKind::Column);
    visitor.end_page(&screen.pages[0]);
    visitor.end_screen(&screen);
    assert!(visitor.stack.is_empty());
    assert!(visitor.output.is_some());
    // Traverse all nested groups/tabs through the object-safe host traversal.
    let mut visitor = IcedControlVisitor::default();
    screen.visit(&mut visitor as &mut dyn ControlVisitor);
    assert!(visitor.stack.is_empty());
}

#[test]
fn native_navigation_emits_typed_activation_and_updates_all_pages_and_tabs() {
    let mut state = fixture(true);
    for page in state.screen().pages {
        let mut element = page_radio(&page.select, false);
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
            matches!(&messages[0], ControlEvent { id: Some(id), input: ControlInput::Activate } if id == &page.select.id)
        );
        for message in messages {
            state.dispatch(message);
        }
        assert!(
            state
                .screen()
                .pages
                .iter()
                .find(|p| p.id == page.id)
                .unwrap()
                .selected
        );
    }
    let mut inventory = Inventory::default();
    state.screen().visit(&mut inventory);
    for tab in inventory.1 {
        send(&mut state, tab.select.id, ControlInput::Activate);
        let mut current = Inventory::default();
        state.screen().visit(&mut current);
        assert!(current.1.iter().find(|t| t.id == tab.id).unwrap().selected);
    }
}

#[test]
fn dynamic_ids_restore_context_before_editing_instead_of_using_selected_target() {
    let mut state = fixture(true);
    let screen = state.screen();
    let mut inventory = Inventory::default();
    screen.visit(&mut inventory);
    let system = inventory
        .1
        .iter()
        .find(|t| t.label.contains("NES"))
        .unwrap()
        .clone();
    let candidate = controls_in(&system.body)
        .into_iter()
        .find(|c| matches!(&c.kind, ControlKind::Choice { choices, .. } if choices.len() > 1))
        .unwrap();
    let ControlKind::Choice { selected, .. } = candidate.kind else {
        unreachable!()
    };
    let index = usize::from(selected == Some(0));
    send(
        &mut state,
        screen.pages[0].select.id,
        ControlInput::Activate,
    );
    send(&mut state, candidate.id, ControlInput::Select(index));
    assert!(
        matches!(state.screen().control(candidate.id).unwrap().kind, ControlKind::Choice { selected: Some(i), .. } if i == index)
    );
    let mut current = Inventory::default();
    state.screen().visit(&mut current);
    assert!(
        current
            .1
            .iter()
            .find(|t| t.id == system.id)
            .unwrap()
            .selected
    );

    // A controller-slot callback must restore its own input tab too, not
    // accidentally edit whichever system/shortcuts tab is currently selected.
    let input = current
        .1
        .iter()
        .find(|tab| {
            controls_in(&tab.body).iter().any(|c| {
                matches!(&c.kind, ControlKind::Choice { choices, .. }
                    if choices.iter().any(|choice| choice.contains("None")))
            })
        })
        .unwrap()
        .clone();
    let slot = controls_in(&input.body)
        .into_iter()
        .find(|c| {
            matches!(&c.kind, ControlKind::Choice { choices, .. }
                if choices.iter().any(|choice| choice.contains("None")))
        })
        .unwrap();
    let ControlKind::Choice { choices, .. } = &slot.kind else {
        unreachable!()
    };
    let none = choices
        .iter()
        .position(|choice| choice.contains("None"))
        .unwrap();
    let shortcuts = current
        .1
        .iter()
        .find(|tab| tab.label.contains("Shortcut"))
        .unwrap();
    send(&mut state, shortcuts.select.id, ControlInput::Activate);
    send(&mut state, slot.id, ControlInput::Select(none));
    assert!(matches!(
        state.screen().control(slot.id).unwrap().kind,
        ControlKind::Choice { selected: Some(index), .. } if index == none
    ));
    let mut current = Inventory::default();
    state.screen().visit(&mut current);
    assert!(
        current
            .1
            .iter()
            .find(|tab| tab.id == input.id)
            .unwrap()
            .selected
    );
}

fn controls_in(layout: &wakui::control::Layout) -> Vec<Control> {
    use wakui::control::Node;
    layout
        .children
        .iter()
        .flat_map(|node| match node {
            Node::Control(c) => vec![c.clone()],
            Node::Row(l) | Node::Column(l) | Node::Group(l) | Node::Table(l) => controls_in(l),
            Node::Tabs(t) => t.tabs.iter().flat_map(|t| controls_in(&t.body)).collect(),
        })
        .collect()
}

#[test]
fn stale_disabled_hidden_wrong_kind_and_missing_callbacks_are_ignored() {
    let mut state = fixture(false);
    let info = controls(&state.screen())
        .into_iter()
        .find(|c| {
            matches!(c.kind, ControlKind::Toggle { .. })
                && c.metadata.accessible_label.contains("Info")
        })
        .unwrap();
    send(&mut state, info.id, ControlInput::Bool(true));
    let callback = Message::new(
        field(&state, UiText::NotificationDuration).id,
        ControlInput::Number(2000),
    );
    send(&mut state, info.id, ControlInput::Bool(false));
    let before = state.screen();
    let foreign = field(&fixture(false), UiText::Language).id;
    for message in [
        callback,
        Message::new(
            field(&state, UiText::MasterVolume).id,
            ControlInput::Number(u32::MAX),
        ),
        Message::new(field(&state, UiText::Mute).id, ControlInput::Select(0)),
        Message::new(
            field(&state, UiText::Language).id,
            ControlInput::Select(usize::MAX),
        ),
        Message::new(foreign, ControlInput::Activate),
        Message::new(
            field(&state, UiText::SaveStorageDirectory).id,
            ControlInput::Text("hidden".into()),
        ),
    ] {
        state.dispatch(message);
        assert_eq!(state.screen(), before);
    }
}

#[test]
fn capture_key_resolves_current_capture_and_clear_reaches_viewmodel() {
    let mut state = fixture(false);
    let id = controls(&state.screen())
        .iter()
        .find(|c| matches!(c.kind, ControlKind::Binding { .. }))
        .unwrap()
        .id;
    state.dispatch(Message::capture_key(Key::KeyZ));
    assert!(state.screen().capture.target.is_none());
    send(&mut state, id, ControlInput::Activate);
    assert_eq!(state.screen().capture.target, Some(id));
    state.dispatch(Message::capture_key(Key::KeyZ));
    assert!(state.screen().capture.target.is_none());
    assert!(matches!(
        &state.screen().control(id).unwrap().kind,
        ControlKind::Binding { text, capturing: false, .. } if text.contains('Z')
    ));
    send(&mut state, id, ControlInput::CaptureClear);
    assert!(matches!(
        &state.screen().control(id).unwrap().kind,
        ControlKind::Binding { text, capturing: false, .. } if !text.contains('Z')
    ));
}

#[test]
fn text_validation_footer_submit_and_cancel_use_model_policy() {
    let mut state = fixture(false);
    let policy = field(&state, UiText::SaveStoragePolicy);
    let ControlKind::Choice { choices, .. } = policy.kind else {
        unreachable!()
    };
    let custom = choices
        .iter()
        .position(|c| c.to_lowercase().contains("custom"))
        .unwrap();
    send(&mut state, policy.id, ControlInput::Select(custom));
    let directory = field(&state, UiText::SaveStorageDirectory).id;
    let submit = field(&state, UiText::Ok).id;
    send(&mut state, directory, ControlInput::Text(String::new()));
    assert!(!state.screen().control(submit).unwrap().metadata.enabled);
    assert!(state.screen().message.is_some());
    send(&mut state, submit, ControlInput::Activate);
    assert!(!state.mailbox().close_requested());
    assert!(!state.mailbox().has_pending());
    let effects = state.dispatch(Message::new(directory, ControlInput::Activate));
    assert!(
        matches!(effects.as_slice(), [wakui::control::ControlEffect::PickDirectory { target, .. }] if *target == directory)
    );
    send(&mut state, directory, ControlInput::Text("/tmp".into()));
    assert!(state.screen().control(submit).unwrap().metadata.enabled);
    send(&mut state, submit, ControlInput::Activate);
    assert!(state.mailbox().close_requested());
    assert!(state.mailbox().has_pending());
    let mut state = fixture(false);
    let cancel = field(&state, UiText::Cancel).id;
    send(&mut state, cancel, ControlInput::Activate);
    assert!(state.mailbox().close_requested());
    assert!(!state.mailbox().has_pending());
}

#[test]
fn numeric_native_slider_honors_model_range_step_and_emits_ids() {
    let state = fixture(false);
    let mut control = field(&state, UiText::NotificationDuration);
    control.metadata.enabled = true;
    control.kind = ControlKind::Number {
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
        [ControlEvent {
            input: ControlInput::Number(20),
            ..
        }]
    ));
}

#[test]
fn disabled_native_controls_never_publish_or_open_overlays() {
    let state = fixture(false);
    for key in [
        UiText::Language,
        UiText::NotificationDuration,
        UiText::Mute,
        UiText::SaveStorageDirectory,
        UiText::Cancel,
    ] {
        let mut control = field(&state, key);
        control.metadata.enabled = false;
        let mut element = control_element(&control, true, iced::Font::default());
        let (renderer, mut tree, node) = native_layout(&mut element);
        let point = match control.kind {
            ControlKind::Choice { .. } | ControlKind::Number { .. } | ControlKind::Text { .. } => {
                Layout::new(&node)
                    .children()
                    .nth(1)
                    .unwrap()
                    .bounds()
                    .center()
            }
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
    let state = fixture(false);
    let mut control = field(&state, UiText::Scaling);
    control.kind = ControlKind::Choice {
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
        [ControlEvent {
            input: ControlInput::Select(1),
            ..
        }]
    ));
}

#[test]
fn native_browse_change_clear_buttons_emit_only_current_control_ids() {
    let state = fixture(false);
    let screen = state.screen();
    let binding = controls(&screen)
        .iter()
        .find(|c| matches!(c.kind, ControlKind::Binding { .. }))
        .unwrap()
        .id;
    for (id, child, expected) in [
        (
            field(&state, UiText::SaveStorageDirectory).id,
            2,
            ControlInput::Activate,
        ),
        (binding, 2, ControlInput::Activate),
        (binding, 3, ControlInput::CaptureClear),
    ] {
        let mut control = screen.control(id).unwrap().clone();
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
            format!("{:?}", [Message::new(id, expected)])
        );
    }
}

#[test]
fn topology_change_rejects_removed_binding_and_keyboard_uses_latest_capture() {
    let mut state = fixture(true);
    let mut inventory = Inventory::default();
    state.screen().visit(&mut inventory);
    let input = inventory.1.iter().find(|t| controls_in(&t.body).iter().any(|c| matches!(&c.kind, ControlKind::Choice { choices, .. } if choices.iter().any(|s| s.contains("None"))))).unwrap().clone();
    let old: Vec<_> = controls_in(&input.body)
        .iter()
        .filter(|c| matches!(c.kind, ControlKind::Binding { .. }))
        .map(|c| c.id)
        .collect();
    for slot in controls_in(&input.body) {
        if let ControlKind::Choice { choices, .. } = &slot.kind
            && let Some(none) = choices.iter().position(|c| c.contains("None"))
        {
            send(&mut state, slot.id, ControlInput::Select(none));
        }
    }
    let removed = old
        .into_iter()
        .find(|id| state.screen().control(*id).is_none())
        .unwrap();
    let before = state.screen();
    send(&mut state, removed, ControlInput::Activate);
    assert_eq!(state.screen(), before);
    assert!(state.screen().capture.target.is_none());
    let bindings: Vec<_> = controls(&state.screen())
        .into_iter()
        .filter(|c| matches!(c.kind, ControlKind::Binding { .. }))
        .collect();
    let first = bindings[0].id;
    let second = bindings[1].id;
    send(&mut state, first, ControlInput::Activate);
    let callback = Message::capture_key(Key::KeyZ);
    send(&mut state, second, ControlInput::Activate);
    state.dispatch(callback);
    assert!(state.screen().capture.target.is_none());
    assert!(matches!(&state.screen().control(second).unwrap().kind,
        ControlKind::Binding { text, capturing: false, .. } if text.contains('Z')));
}
