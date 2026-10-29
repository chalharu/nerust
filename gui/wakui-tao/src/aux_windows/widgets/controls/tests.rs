use std::sync::{Arc, atomic::Ordering};

use iced::{
    Event, Point, Rectangle, Size,
    advanced::{
        Layout, Shell, Widget, layout, mouse, renderer,
        widget::{self, Tree},
    },
    keyboard::{self, key},
};
use wakui_core::scene::control::{
    ColumnPolicy, Container, Control, ControlDisplay, ControlEvent, ControlId, ControlMetadata,
    ControlScreen, ControlVisitor, Page, RowPolicy, Tab, TablePolicy,
};
use wakui_core::scene::leaf::{
    ActionLeaf, BindingLeaf, ChoiceLeaf, ImageLeaf, NoteLeaf, NumberLeaf, TextLeaf, ToggleLeaf,
};

use super::super::bindings::tests::{controls, find, prepare};
use crate::control_binding::{El, Message};
use crate::control_renderer::gate;
use crate::control_renderer::{build_screen, control_element};
use crate::event_loop::host::HostState;
use crate::testkit::TestAux;
use wakui_core::chrome::aux::AuxId;
use wakui_core::scene::control::ControlInput;

struct Fixture {
    host: HostState<TestAux>,
    id: AuxId,
}

fn fixture() -> Fixture {
    let host = crate::event_loop::host::TestHost::new().host;
    let mut host = host;
    let id = prepare(&mut host, crate::testkit::TestAux::Alpha);
    Fixture { host, id }
}

impl Fixture {
    fn screen(&self) -> wakui_core::scene::control::ControlScreen {
        self.host.core.widget_screen(self.id).expect("screen")
    }

    fn send(&mut self, id: ControlId, input: ControlInput) {
        let _ = self
            .host
            .core
            .widget_dispatch(self.id, Message::new(id, input));
    }

    fn dispatch(&mut self, event: ControlEvent) -> Vec<wakui_core::scene::control::ControlEffect> {
        self.host.core.widget_dispatch(self.id, event)
    }

    fn field(&self, label: &str) -> Control {
        find(&self.screen(), label)
    }

    fn window(&self) -> wakui_core::chrome::window::Window {
        self.host.aux.window(self.id).expect("window").clone()
    }
}

#[test]
fn outer_window_consumes_the_actual_screen_and_program_title_uses_window_title() {
    let mut fix = fixture();
    let page = fix.screen().pages[3].clone();
    fix.send(page.select.id, ControlInput::Activate);
    // The app-side screen updates synchronously through the dispatch;
    // the registry snapshot refreshes on the next widget pump.
    let screen = fix.screen();
    assert!(
        screen
            .pages
            .iter()
            .any(|current| current.selected && current.id == page.id)
    );
    let window = fix.window();
    let mut element = crate::control_renderer::build(&window);
    let (_, tree, _) = native_layout(&mut element);
    assert!(!tree.children.is_empty());
    // The title port reads the common window, not a standalone screen title.
    assert_eq!(fix.screen().title, window.title);
    // A native tree owns all text even after the window is dropped.
    drop(window);
    let (_, tree, _) = native_layout(&mut element);
    assert!(!tree.children.is_empty());
}

fn native_layout(element: &mut El<'static>) -> (iced_tiny_skia::Renderer, Tree, layout::Node) {
    let renderer =
        iced_tiny_skia::Renderer::new(crate::input::convert::default_font(), iced::Pixels(16.0));
    let mut tree = Tree::new(&*element);
    let node = element.as_widget_mut().layout(
        &mut tree,
        &renderer,
        &layout::Limits::new(Size::ZERO, Size::new(960.0, 720.0)),
    );
    (renderer, tree, node)
}

// Domain-free scenes: no history/settings presenter or content-specific widths.
fn table_layout(children: Vec<wakui_core::scene::control::Node>) -> Container {
    plain_layout(children, 8.0, 0.0, Box::new(ColumnPolicy))
}

fn table_grid(
    children: Vec<wakui_core::scene::control::Node>,
    spacing: f64,
    indent: f64,
) -> Container {
    plain_layout(children, spacing, indent, Box::new(TablePolicy))
}

fn plain_layout(
    children: Vec<wakui_core::scene::control::Node>,
    spacing: f64,
    indent: f64,
    policy: Box<dyn wakui_core::scene::control::LayoutPolicy>,
) -> Container {
    let mut ids = wakui_core::scene::control::IdRegistry::default();
    let mut metadata = ControlMetadata::new("layout");
    metadata.hints.spacing = Some(spacing);
    metadata.hints.indent = Some(indent);
    Container {
        id: ids.id(()),
        label: String::new(),
        metadata,
        header: None,
        policy,
        children,
    }
}

fn row_layout(
    children: Vec<wakui_core::scene::control::Node>,
    spacing: f64,
    indent: f64,
) -> Container {
    plain_layout(children, spacing, indent, Box::new(RowPolicy))
}

fn table_note(value: &str, heading: bool) -> Control {
    let mut ids = wakui_core::scene::control::IdRegistry::default();
    Control {
        id: ids.id(()),
        label: String::new(),
        metadata: ControlMetadata::new(value),
        display: if heading {
            ControlDisplay::Heading
        } else {
            ControlDisplay::Note
        },
        leaf: Box::new(NoteLeaf { text: value.into() }),
    }
}

fn table_row(values: &[&str], heading: bool) -> wakui_core::scene::control::Node {
    use wakui_core::scene::control::Node;
    Node::Container(Box::new(row_layout(
        values
            .iter()
            .map(|value| Node::Control(table_note(value, heading)))
            .collect(),
        8.0,
        0.0,
    )))
}

fn table_screen(body: wakui_core::scene::control::Container) -> ControlScreen {
    let mut ids = wakui_core::scene::control::IdRegistry::default();
    ControlScreen {
        title: "Generic controls".into(),
        pages: vec![Page {
            id: ids.id("page"),
            label: "Page".into(),
            selected: true,
            select: Control {
                leaf: Box::new(ActionLeaf),
                ..table_note("Page", false)
            },
            body,
        }],
        footer: table_layout(vec![]),
        message: None,
        capture: wakui_core::scene::control::Capture {
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
    use wakui_core::scene::control::Node;
    let long = "a long message with variable content ".repeat(80);
    let mut table = table_grid(
        vec![
            table_row(&["Time", "Severity", "Source", "Message"], true),
            table_row(&["12:34:56.789", "I", "subsystem", "short"], false),
            table_row(&["1", "Warning", "X", &long], false),
            table_row(&["ragged", "row"], false),
        ],
        8.0,
        0.0,
    );
    table.label = "Table title".into();
    table.header = Some(table_note("Table description", false));
    table.metadata.hints.indent = Some(11.0);
    let screen = table_screen(table_layout(vec![Node::Container(Box::new(table))]));
    for width in [960.0, 640.0] {
        let mut element = build_screen(&screen);
        let renderer = iced_tiny_skia::Renderer::new(
            crate::input::convert::default_font(),
            iced::Pixels(16.0),
        );
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
    use wakui_core::scene::control::Node;
    let mut action = table_note("Action", false);
    action.label = "Action".into();
    action.leaf = Box::new(ActionLeaf);
    let mut inner = table_grid(
        vec![table_row(&["nested", "table", "cells"], false)],
        8.0,
        0.0,
    );
    inner.header = Some(table_note("Nested header", true));
    let mut hidden = table_grid(vec![table_row(&["hidden"; 4], false)], 8.0, 0.0);
    hidden.metadata.visible = false;
    let mut disabled_row = row_layout(
        vec![
            Node::Control(action.clone()),
            Node::Container(Box::new(plain_layout(
                vec![Node::Container(Box::new(inner))],
                8.0,
                0.0,
                Box::new(ColumnPolicy),
            ))),
            Node::Control(table_note("third", false)),
            Node::Control(table_note("fourth", false)),
        ],
        13.0,
        9.0,
    );
    disabled_row.metadata.enabled = false;
    disabled_row.metadata.hints.indent = Some(9.0);
    disabled_row.metadata.hints.spacing = Some(13.0);
    let mut table = table_grid(
        vec![
            table_row(&["one", "two", "three", "four"], true),
            Node::Container(Box::new(disabled_row)),
            Node::Container(Box::new(hidden)),
        ],
        17.0,
        5.0,
    );
    table.header = Some(table_note("Visible table header", true));
    table.metadata.hints.spacing = Some(17.0);
    table.metadata.hints.indent = Some(5.0);
    let mut element = build_screen(&table_screen(table_layout(vec![Node::Container(
        Box::new(table.clone()),
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
    let Node::Container(row) = &mut table.children[1] else {
        unreachable!()
    };
    row.metadata.enabled = true;
    let mut element = build_screen(&table_screen(table_layout(vec![Node::Container(
        Box::new(table.clone()),
    )])));
    let (renderer, mut tree, node) = native_layout(&mut element);
    assert_eq!(
        feed(&mut element, &renderer, &mut tree, &node, &click(), point),
        vec![Message::new(action.id, ControlInput::Activate)]
    );

    table.metadata.visible = false;
    let mut element = build_screen(&table_screen(table_layout(vec![Node::Container(
        Box::new(table),
    )])));
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
    let fix = fixture();
    let mut events = keys(
        keyboard::Key::Named(key::Named::ArrowUp),
        key::Code::ArrowUp,
        None,
    )
    .to_vec();
    // Positive control: these are real slider keyboard events, not just modifier updates.
    let mut control = fix.field("Number A");
    control.metadata.enabled = true;
    control.leaf = Box::new(NumberLeaf {
        value: 10,
        min: 10,
        max: 30,
        step: 5,
        unit: "ticks".into(),
        formatted: "10 ticks".into(),
    });
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
    for key in ["Choice A", "Number A", "Toggle A", "Text A", "Cancel"] {
        let mut control = fix.field(key);
        control.metadata.enabled = false;
        let mut element = control_element(&control, true, iced::Font::default());
        let (renderer, mut tree, node) = native_layout(&mut element);
        let leafy = control.downcast::<ChoiceLeaf>().is_some()
            || control.downcast::<NumberLeaf>().is_some()
            || control.downcast::<TextLeaf>().is_some();
        let point = if leafy {
            Layout::new(&node)
                .children()
                .nth(1)
                .unwrap()
                .bounds()
                .center()
        } else {
            Layout::new(&node).bounds().center()
        };
        assert_ignored(&mut element, &renderer, &mut tree, &node, &events, point);
    }
}

#[test]
fn disabling_an_open_native_pick_list_removes_overlay_and_ignores_input() {
    let fix = fixture();
    // Exercise both the control's own flag and the enclosing layout's flag.
    for parent_enabled in [true, false] {
        let mut control = fix.field("Choice C");
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
    let fix = fixture();
    for parent_enabled in [true, false] {
        let mut control = fix.field("Number A");
        control.metadata.enabled = true;
        control.leaf = Box::new(NumberLeaf {
            value: 10,
            min: 10,
            max: 30,
            step: 5,
            unit: "ticks".into(),
            formatted: "10 ticks".into(),
        });
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
    let fix = fixture();
    let mut control = fix.field("Text A");
    control.metadata.enabled = true;
    for enabled in [true, false] {
        let mut element = gate(
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
        let mut element = gate(
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
    let fix = fixture();
    let mut control = fix.field("Text A");
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
    {
        let mut fix = fixture();
        let language = fix.field("Choice A");
        fix.send(language.id, ControlInput::Select(2));
        let pages = fix.screen().pages;
        assert_eq!(pages.len(), 6);
        for page in pages {
            fix.send(page.select.id, ControlInput::Activate);
            let before = fix.screen();
            // Temporary screen is gone before iced even performs layout.
            let mut element: El<'static> = crate::control_renderer::build(&fix.window());
            let (_, tree, node) = native_layout(&mut element);
            assert!(!tree.children.is_empty());
            assert!(node.size().width > 0.0 && node.size().height > 0.0);
            assert_eq!(fix.screen(), before);
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
    let fix = fixture();
    let mut inventory = Inventory::default();
    fix.screen().visit(&mut inventory);
    let mut controls = inventory.0;
    // Aux screens carry no raster images; synthesize one so the
    // generic Image path is covered without a Delta fixture.
    controls.push(Control {
        id: ControlId::fresh(),
        label: "Note".to_string(),
        metadata: ControlMetadata::new("Tiles"),
        display: ControlDisplay::Standard,
        leaf: Box::new(ImageLeaf {
            width: 2,
            height: 2,
            rgba: vec![0xff; 2 * 2 * 4],
        }),
    });
    let mut kinds = [false; 8];
    for mut control in controls {
        let index = if control.downcast::<ToggleLeaf>().is_some() {
            0
        } else if control.downcast::<ChoiceLeaf>().is_some() {
            1
        } else if control.downcast::<NumberLeaf>().is_some() {
            2
        } else if control.downcast::<TextLeaf>().is_some() {
            3
        } else if control.downcast::<BindingLeaf>().is_some() {
            4
        } else if control.downcast::<ActionLeaf>().is_some() {
            5
        } else if control.downcast::<NoteLeaf>().is_some() {
            6
        } else if control.downcast::<ImageLeaf>().is_some() {
            7
        } else {
            panic!("uncovered leaf");
        };
        kinds[index] = true;
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
fn stale_disabled_hidden_wrong_kind_and_missing_callbacks_are_ignored() {
    let mut fix = fixture();
    let info = controls(&fix.screen())
        .into_iter()
        .find(|c| c.downcast::<ToggleLeaf>().is_some())
        .unwrap();
    fix.send(info.id, ControlInput::Bool(true));
    let callback = Message::new(fix.field("Number A").id, ControlInput::Number(2000));
    fix.send(info.id, ControlInput::Bool(false));
    let before = fix.screen();
    let foreign = {
        let other = fixture();
        other.field("Choice A").id
    };
    for message in [
        callback,
        Message::new(fix.field("Number A").id, ControlInput::Number(u32::MAX)),
        Message::new(fix.field("Toggle A").id, ControlInput::Select(0)),
        Message::new(fix.field("Choice A").id, ControlInput::Select(usize::MAX)),
        Message::new(foreign, ControlInput::Activate),
        Message::new(ControlId::fresh(), ControlInput::Activate),
    ] {
        fix.dispatch(message);
        assert_eq!(fix.screen(), before);
    }
}

#[test]
fn numeric_native_slider_honors_model_range_step_and_emits_ids() {
    let fix = fixture();
    let mut control = fix.field("Number A");
    control.metadata.enabled = true;
    control.leaf = Box::new(NumberLeaf {
        value: 10,
        min: 10,
        max: 30,
        step: 5,
        unit: "ticks".into(),
        formatted: "10 ticks".into(),
    });
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
    let fix = fixture();
    for key in ["Choice A", "Number A", "Toggle A", "Text A", "Cancel"] {
        let mut control = fix.field(key);
        control.metadata.enabled = false;
        let mut element = control_element(&control, true, iced::Font::default());
        let (renderer, mut tree, node) = native_layout(&mut element);
        let leafy = control.downcast::<ChoiceLeaf>().is_some()
            || control.downcast::<NumberLeaf>().is_some()
            || control.downcast::<TextLeaf>().is_some();
        let point = if leafy {
            Layout::new(&node)
                .children()
                .nth(1)
                .unwrap()
                .bounds()
                .center()
        } else {
            Layout::new(&node).bounds().center()
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
    let fix = fixture();
    let mut control = fix.field("Choice C");
    control.leaf = Box::new(ChoiceLeaf {
        choices: vec!["same".into(), "same".into()],
        selected: Some(0),
    });
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
    let fix = fixture();
    let screen = fix.screen();
    let binding = controls(&screen)
        .iter()
        .find(|c| c.downcast::<BindingLeaf>().is_some())
        .unwrap()
        .id;
    for (id, child, expected) in [
        (fix.field("Text A").id, 2, ControlInput::Activate),
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
