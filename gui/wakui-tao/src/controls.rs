//! Domain-free immediate-mode interpreter of host-owned control scenes.

use iced::{
    Alignment, Length,
    widget::{
        button, checkbox, column, container, image as iced_image, mouse_area, pick_list, radio,
        row, scrollable, slider, text, text_input,
    },
};
use wakui::{
    control::{
        Container, Control, ControlDisplay, ControlInput, ControlMetadata, ControlScreen,
        ControlVisitor, GroupPolicy, Page, RowPolicy, Tab, TablePolicy, Tabs, policy,
    },
    leaf::{
        ActionLeaf, BindingLeaf, ChoiceLeaf, ImageLeaf, ListLeaf, NoteLeaf, NumberLeaf, TextLeaf,
        ToggleLeaf,
    },
    mapper::{PropertyUpdate, map_updates},
};

use crate::binding::{El, Message};

/// All labels and callback IDs are owned; the snapshot may be dropped.
#[cfg(test)]
pub fn build(window: &wakui::window::Window) -> El<'static> {
    let mut visitor = ControlsWindowVisitor::default();
    window.visit(&mut visitor);
    visitor
        .output
        .expect("widget window must contain a control screen")
}

#[cfg(test)]
#[derive(Default)]
struct ControlsWindowVisitor {
    output: Option<El<'static>>,
}

#[cfg(test)]
impl wakui::window::WindowVisitor for ControlsWindowVisitor {
    fn visit_controls(&mut self, screen: &ControlScreen) {
        self.output = Some(build_screen(screen));
    }
}

pub fn build_screen(screen: &ControlScreen) -> El<'static> {
    let mut visitor = IcedControlVisitor::default();
    screen.visit(&mut visitor as &mut dyn ControlVisitor);
    visitor.output.expect("control traversal must finish")
}

#[derive(Default)]
struct Frame {
    visible: bool,
    enabled: bool,
    children: Vec<El<'static>>,
    navigation: Vec<El<'static>>,
    table: Option<Vec<TableChild>>,
}

enum TableChild {
    Element(El<'static>),
    Row {
        cells: Vec<El<'static>>,
        spacing: u16,
        indent: u16,
    },
}

#[derive(Default)]
struct IcedControlVisitor {
    stack: Vec<Frame>,
    sidebar: Vec<El<'static>>,
    page: Option<El<'static>>,
    output: Option<El<'static>>,
    font: iced::Font,
}

impl IcedControlVisitor {
    fn push(&mut self, visible: bool, enabled: bool) {
        let parent = self.stack.last();
        self.stack.push(Frame {
            visible: visible && parent.is_none_or(|f| f.visible),
            enabled: enabled && parent.is_none_or(|f| f.enabled),
            ..Frame::default()
        });
    }

    fn push_metadata(&mut self, metadata: &ControlMetadata) {
        self.push(metadata.visible, metadata.enabled);
    }

    fn append(&mut self, element: El<'static>) {
        let frame = self.stack.last_mut().expect("control frame");
        if frame.visible {
            if let Some(table) = &mut frame.table {
                table.push(TableChild::Element(element));
            } else {
                frame.children.push(element);
            }
        }
    }

    fn finish_container(&mut self, container: &Container) {
        let mut frame = self.stack.pop().expect("container frame");
        let layout = container;
        if !frame.visible {
            return;
        }
        if let Some(table) = frame.table {
            self.append(table_element(table, layout, self.font));
            return;
        }
        if frame.children.is_empty() && layout.label.is_empty() {
            return;
        }
        let spacing = f32::from(layout.spacing);
        let element = if policy::<RowPolicy>(container).is_some() {
            if !layout.label.is_empty() {
                frame
                    .children
                    .insert(0, text(layout.label.clone()).font(self.font).into());
            }
            if let Some(table) = self.stack.last_mut().and_then(|frame| frame.table.as_mut()) {
                table.push(TableChild::Row {
                    cells: frame.children,
                    spacing: layout.spacing,
                    indent: layout.indent,
                });
                return;
            }
            row(frame.children)
                .spacing(spacing)
                .align_y(Alignment::Center)
                .padding(iced::padding::left(f32::from(layout.indent)))
                .into()
        } else if policy::<GroupPolicy>(container).is_some() {
            // Title stands alone; the group header action (e.g. the
            // notification Enable toggle) renders as the first indented
            // row below it, matching GTK's header-below-title order.
            let mut stacked: Vec<El<'static>> = Vec::new();
            if !layout.label.is_empty() {
                stacked.push(
                    row![text(layout.label.clone()).font(iced::Font {
                        weight: iced::font::Weight::Bold,
                        ..self.font
                    })]
                    .spacing(spacing)
                    .align_y(Alignment::Center)
                    .into(),
                );
            }
            stacked.push(
                column(frame.children)
                    .spacing(spacing)
                    .padding(iced::padding::left(f32::from(layout.indent)))
                    .into(),
            );
            column(stacked).spacing(spacing).into()
        } else {
            let mut children = column(frame.children).spacing(spacing);
            if !layout.label.is_empty() {
                children =
                    column![text(layout.label.clone()).font(self.font), children].spacing(spacing);
            }
            children
                .padding(iced::padding::left(f32::from(layout.indent)))
                .into()
        };
        self.append(element);
    }
}

fn table_element(children: Vec<TableChild>, layout: &Container, font: iced::Font) -> El<'static> {
    // All rows use the same tracks, including short/ragged rows. Gutter and
    // indentation are shared minima, never content-dependent shrink widths.
    let (columns, spacing, indent) = children.iter().fold((0, 0, 0), |sizes, child| {
        if let TableChild::Row {
            cells,
            spacing,
            indent,
        } = child
        {
            (
                sizes.0.max(cells.len()),
                sizes.1.max(*spacing),
                sizes.2.max(*indent),
            )
        } else {
            sizes
        }
    });
    let rows = children.into_iter().map(|child| match child {
        TableChild::Element(element) => element,
        TableChild::Row { mut cells, .. } => {
            cells.resize_with(columns, || iced::widget::Space::new().into());
            row(cells
                .into_iter()
                .map(|cell| container(cell).width(Length::FillPortion(1)).into()))
            .width(Length::Fill)
            .spacing(f32::from(spacing))
            .padding(iced::padding::left(f32::from(indent)))
            .align_y(Alignment::Center)
            .into()
        }
    });
    let mut body = column(rows)
        .width(Length::Fill)
        .spacing(f32::from(layout.spacing));
    if !layout.label.is_empty() {
        body = column![text(layout.label.clone()).font(font), body]
            .width(Length::Fill)
            .spacing(f32::from(layout.spacing));
    }
    body.padding(iced::padding::left(f32::from(layout.indent)))
        .into()
}

impl ControlVisitor for IcedControlVisitor {
    fn begin_screen(&mut self, _screen: &ControlScreen) {
        self.font = crate::convert::default_font();
        self.push(true, true);
    }

    fn end_screen(&mut self, screen: &ControlScreen) {
        let frame = self.stack.pop().expect("screen frame");
        let content = self.page.take().unwrap_or_else(|| column![].into());
        // No in-content title label (GTK parity: the outer window carries
        // the screen title). Single-panel screens (e.g. notification
        // history) need no navigation chrome; content fills the dialog.
        let body: El<'static> = if self.sidebar.len() > 1 {
            row![
                container(column(std::mem::take(&mut self.sidebar)).spacing(10))
                    .padding(12)
                    .width(Length::Fixed(180.0)),
                scrollable(container(content).padding(12).width(Length::Fill))
                    .width(Length::Fill)
                    .height(Length::Fill)
            ]
            .spacing(16)
            .height(Length::Fill)
            .into()
        } else {
            self.sidebar.clear();
            scrollable(container(content).padding(12).width(Length::Fill))
                .width(Length::Fill)
                .height(Length::Fill)
                .into()
        };
        let mut root = column![body].spacing(12).padding(16).height(Length::Fill);
        if let Some(message) = &screen.message {
            root = root.push(text(message.clone()).font(self.font));
        }
        for footer in frame.children {
            root = root.push(row![iced::widget::Space::new().width(Length::Fill), footer]);
        }
        self.output = Some(root.into());
    }

    fn begin_page(&mut self, page: &Page) {
        if page.select.metadata.visible {
            self.sidebar.push(page_radio(&page.select, page.selected));
        }
        self.push(
            page.selected && page.select.metadata.visible,
            page.select.metadata.enabled,
        );
    }

    fn end_page(&mut self, _: &Page) {
        let frame = self.stack.pop().expect("page frame");
        if frame.visible {
            self.page = Some(column(frame.children).into());
        }
    }

    fn begin_container(&mut self, container: &Container) {
        self.push_metadata(&container.metadata);
        if policy::<TablePolicy>(container).is_some() {
            self.stack.last_mut().expect("table frame").table = Some(Vec::new());
        }
    }
    fn end_container(&mut self, container: &Container) {
        self.finish_container(container);
    }

    fn begin_tabs(&mut self, tabs: &Tabs) {
        self.push_metadata(&tabs.metadata);
    }
    fn end_tabs(&mut self, _: &Tabs) {
        let frame = self.stack.pop().expect("tabs frame");
        if frame.visible {
            // Segmented control: borderless segments in one bordered
            // track; the selected segment carries the primary fill.
            let track = row(frame.navigation).spacing(0);
            let track = container(track).padding(2).style(container::bordered_box);
            self.append(column![track, column(frame.children)].spacing(12).into());
        }
    }
    fn begin_tab(&mut self, tab: &Tab) {
        let parent = self.stack.last_mut().expect("tabs frame");
        if parent.visible && tab.select.metadata.visible {
            parent
                .navigation
                .push(tab_segment(&tab.select, tab.selected, parent.enabled));
        }
        self.push(
            tab.selected && tab.select.metadata.visible,
            tab.select.metadata.enabled,
        );
    }
    fn end_tab(&mut self, _: &Tab) {
        let frame = self.stack.pop().expect("tab frame");
        if frame.visible {
            self.append(column(frame.children).into());
        }
    }
    fn control(&mut self, control: &Control) {
        let frame = self.stack.last().expect("control frame");
        if frame.visible && control.metadata.visible {
            self.append(control_element(control, frame.enabled, self.font));
            // Container-like leaves degrade flat in immediate mode.
            // Base leaves no-op here.
            let ctx = wakui::leaf::LeafCtx {
                id: control.id,
                hints: control.metadata.hints,
            };
            control.leaf.walk(&ctx, self);
        }
    }
}

/// Framework-mapper chrome snapshot: enabled, label, display.
/// Kind drives construction; accessibility/help stay unread (no hover).
fn chrome(control: &Control) -> (bool, String, ControlDisplay) {
    let mut enabled = true;
    let mut label = control.label.clone();
    let mut display = control.display;
    for update in map_updates(None, control) {
        match update {
            PropertyUpdate::SetEnabled(v) => enabled = v,
            PropertyUpdate::SetLabel(t) => label = t,
            PropertyUpdate::SetDisplay(d) => display = d,
            _ => {}
        }
    }
    (enabled, label, display)
}

/// Page selector: radio group, one dot per page. Master parity;
/// page selects are always enabled, so the radio is unconditional
/// and re-selecting the current page stays a harmless no-op.
/// Single page-select radio. Hosts reuse this for custom sidebars; the
/// settings sidebar builds one per page through [`build_screen`].
pub fn page_radio(control: &Control, selected: bool) -> El<'static> {
    let id = control.id;
    let (_, label, _) = chrome(control);
    radio(label.to_owned(), id, selected.then_some(id), move |id| {
        Message::new(id, ControlInput::Activate)
    })
    .into()
}

/// Sub-tab segment: borderless text inside the bordered track, primary
/// fill when selected. Deliberately not master blue pills; the track
/// reads as one segmented control instead of scattered buttons.
fn tab_segment(control: &Control, selected: bool, enabled: bool) -> El<'static> {
    let (own_enabled, label, _) = chrome(control);
    button(text(label.to_owned()))
        .style(if selected {
            button::primary
        } else {
            button::text
        })
        .on_press_maybe(
            (enabled && own_enabled && !selected)
                .then(|| Message::new(control.id, ControlInput::Activate)),
        )
        .into()
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Choice {
    index: usize,
    label: String,
}

impl std::fmt::Display for Choice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.label)
    }
}

/// Render one retained control outside a full screen. Native hosts use
/// this for focused previews; tests use it to assert per-kind output.
pub fn control_element(control: &Control, parent_enabled: bool, font: iced::Font) -> El<'static> {
    // All chrome decisions flow through the framework mapper; the kind
    // drives construction. Immediate mode has no patch step, so the full
    // list is folded here. Accessibility/help stay unread: Tao has no hover
    // affordance (GTK parity: tooltip only).
    let (own_enabled, label, display) = chrome(control);
    let enabled = parent_enabled && own_enabled;
    let id = control.id;
    let input = move |value| Message::new(id, value);
    let label_text = || text(label.clone()).font(font).width(Length::Fixed(220.0));
    let element: El<'static> = if let Some(leaf) = control.downcast::<ToggleLeaf>() {
        checkbox(leaf.value)
            .label(label.to_owned())
            .on_toggle_maybe(enabled.then_some(move |v| input(ControlInput::Bool(v))))
            .into()
    } else if let Some(leaf) = control.downcast::<ChoiceLeaf>() {
        let options: Vec<_> = leaf
            .choices
            .iter()
            .enumerate()
            .map(|(index, label)| Choice {
                index,
                label: label.clone(),
            })
            .collect();
        let selected = leaf.selected.and_then(|i| options.get(i)).cloned();
        let pick = pick_list(options, selected, move |v: Choice| {
            input(ControlInput::Select(v.index))
        })
        .width(Length::Fill)
        .font(font)
        .style(move |theme, status| {
            let mut style = pick_list::default(
                theme,
                if enabled {
                    status
                } else {
                    pick_list::Status::Active
                },
            );
            if !enabled {
                style.text_color = theme.extended_palette().secondary.base.color;
                style.handle_color = style.text_color;
            }
            style
        });
        row![label_text(), crate::disabled::gate(pick.into(), enabled)]
            .spacing(12)
            .into()
    } else if let Some(leaf) = control.downcast::<NumberLeaf>() {
        let native = slider(leaf.min..=leaf.max, leaf.value, move |v| {
            input(ControlInput::Number(v))
        })
        .step(leaf.step)
        .style(move |theme, status| {
            let mut style = slider::default(
                theme,
                if enabled {
                    status
                } else {
                    slider::Status::Active
                },
            );
            if !enabled {
                let color = theme.extended_palette().secondary.base.color;
                style.rail.backgrounds.0 = color.into();
                style.handle.background = color.into();
            }
            style
        });
        row![
            label_text(),
            crate::disabled::gate(native.into(), enabled),
            text(leaf.formatted.clone()).font(font)
        ]
        .spacing(12)
        .align_y(Alignment::Center)
        .into()
    } else if let Some(leaf) = control.downcast::<TextLeaf>() {
        let edit_id = control.id;
        let edit = text_input(&leaf.placeholder, &leaf.value)
            .font(font)
            .width(Length::Fill)
            .on_input_maybe(
                enabled.then_some(move |v| Message::new(edit_id, ControlInput::Text(v))),
            );
        let mut items = row![label_text(), crate::disabled::gate(edit.into(), enabled)]
            .spacing(12)
            .align_y(Alignment::Center);
        if let Some(browse) = &leaf.browse_label {
            items = items.push(
                button(text(browse.clone()).font(font))
                    .on_press_maybe(enabled.then(|| input(ControlInput::Activate))),
            );
        }
        items.into()
    } else if let Some(leaf) = control.downcast::<BindingLeaf>() {
        row![
            label_text(),
            text(leaf.text.clone()).font(font).width(Length::Fill),
            button(text(leaf.change_label.clone()).font(font))
                .on_press_maybe(enabled.then(|| input(ControlInput::Activate))),
            button(text(leaf.clear_label.clone()).font(font))
                .on_press_maybe(enabled.then(|| input(ControlInput::CaptureClear)))
        ]
        .spacing(12)
        .align_y(Alignment::Center)
        .into()
    } else if control.downcast::<ActionLeaf>().is_some() {
        button(text(label.to_owned()).font(font))
            .style(if display == ControlDisplay::Primary {
                button::primary
            } else {
                button::secondary
            })
            .on_press_maybe(enabled.then(|| input(ControlInput::Activate)))
            .into()
    } else if let Some(leaf) = control.downcast::<NoteLeaf>() {
        let note = text(leaf.text.clone())
            .font(font)
            .size(if display == ControlDisplay::Heading {
                16
            } else {
                13
            });
        if label.is_empty() {
            note.into()
        } else {
            row![label_text(), note].spacing(12).into()
        }
    } else if let Some(leaf) = control.downcast::<ImageLeaf>() {
        let handle =
            iced::widget::image::Handle::from_rgba(leaf.width, leaf.height, leaf.rgba.clone());
        let shown = iced_image(handle);
        // Hover positions arrive as image pixels; floor the float point.
        mouse_area(shown)
            .on_move(move |point| {
                input(ControlInput::Hover {
                    x: point.x.max(0.0) as u32,
                    y: point.y.max(0.0) as u32,
                })
            })
            .into()
    } else if control.downcast::<ListLeaf>().is_some() {
        // Immediate mode degrades lists flat: rows arrive through the
        // explicit walk in `control()` as sibling notes.
        iced::widget::Space::new().width(Length::Shrink).into()
    } else {
        // Unknown future leaf: read-only label with the pre-localized
        // control label. Never fail construction.
        text(label.to_owned()).font(font).into()
    };
    // Help surfaces as hover tooltips in GTK; Tao has no hover affordance,
    // so a permanent subtitle (e.g. "Occupied by another slot") only adds
    // noise. The disabled state already reads off the dimmed input itself.
    element
}

#[cfg(test)]
mod tests {
    use super::*;
    use iced::{
        Event, Point, Rectangle, Size,
        advanced::{Shell, layout, mouse, widget::Tree},
    };
    use wakui::control::{
        Capture, ColumnPolicy, Container, ControlDisplay, ControlEvent, ControlId, ControlMetadata,
        Id, Node, Page,
    };
    use wakui::leaf::ActionLeaf;

    fn action_control(label: &str) -> Control {
        Control {
            id: Id::fresh(),
            label: label.to_string(),
            metadata: ControlMetadata::new(label),
            display: ControlDisplay::Standard,
            leaf: Box::new(ActionLeaf),
        }
    }

    fn group(children: Vec<Control>) -> Container {
        Container {
            id: Id::fresh(),
            label: String::new(),
            metadata: ControlMetadata::new("group"),
            spacing: 0,
            indent: 0,
            header: None,
            policy: Box::new(GroupPolicy),
            children: children.into_iter().map(Node::Control).collect(),
        }
    }

    fn page(label: &str, selected: bool, body: Container) -> Page {
        Page {
            id: Id::fresh(),
            label: label.to_string(),
            selected,
            select: action_control(label),
            body,
        }
    }

    fn screen(pages: Vec<Page>) -> ControlScreen {
        ControlScreen {
            title: "Test".to_string(),
            pages,
            footer: Container {
                id: Id::fresh(),
                label: String::new(),
                metadata: ControlMetadata::new("footer"),
                spacing: 0,
                indent: 0,
                header: None,
                policy: Box::new(ColumnPolicy),
                children: Vec::new(),
            },
            message: None,
            capture: Capture {
                target: None,
                prompt: String::new(),
            },
        }
    }

    #[test]
    fn visitor_balances_hidden_disabled_frames() {
        let screen = screen(vec![
            page("First", true, group(vec![action_control("Mute")])),
            page("Second", false, group(vec![action_control("OK")])),
        ]);
        let mut visitor = IcedControlVisitor::default();
        visitor.begin_screen(&screen);
        visitor.begin_page(&screen.pages[0]);
        let mut group = screen.pages[0].body.clone();
        group.metadata.visible = false;
        visitor.begin_container(&group);
        visitor.control(&action_control("Mute"));
        assert!(visitor.stack.last().unwrap().children.is_empty());
        visitor.end_container(&group);
        assert!(visitor.stack.last().unwrap().children.is_empty());
        group.metadata.visible = true;
        group.metadata.enabled = false;
        visitor.begin_container(&group);
        assert!(!visitor.stack.last().unwrap().enabled);
        visitor.begin_container(&group);
        assert!(!visitor.stack.last().unwrap().enabled);
        visitor.control(&action_control("Mute"));
        visitor.end_container(&group);
        visitor.end_container(&group);
        visitor.end_page(&screen.pages[0]);
        visitor.end_screen(&screen);
        assert!(visitor.stack.is_empty());
        assert!(visitor.output.is_some());
        // Traverse all nested groups through the object-safe traversal.
        let mut visitor = IcedControlVisitor::default();
        screen.visit(&mut visitor as &mut dyn ControlVisitor);
        assert!(visitor.stack.is_empty());
    }

    fn native_layout(element: &mut El<'static>) -> (iced_tiny_skia::Renderer, Tree, layout::Node) {
        let renderer = iced_tiny_skia::Renderer::new(iced::Font::default(), iced::Pixels(16.0));
        let mut tree = Tree::new(&*element);
        let node = element.as_widget_mut().layout(
            &mut tree,
            &renderer,
            &layout::Limits::new(Size::ZERO, Size::new(960.0, 720.0)),
        );
        (renderer, tree, node)
    }

    fn click_messages(element: &mut El<'static>, target: ControlId) -> Vec<Message> {
        let (renderer, mut tree, node) = native_layout(element);
        let mut messages = vec![];
        let mut clipboard = iced::advanced::clipboard::Null;
        for event in [
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)),
            Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)),
        ] {
            element.as_widget_mut().update(
                &mut tree,
                &event,
                layout::Layout::new(&node),
                mouse::Cursor::Available(Point::new(10.0, 10.0)),
                &renderer,
                &mut clipboard,
                &mut Shell::new(&mut messages),
                &Rectangle::with_size(Size::new(960.0, 720.0)),
            );
        }
        assert!(
            messages.iter().all(|message| message.id == Some(target)),
            "clicks route to the pressed control"
        );
        messages
    }

    #[test]
    fn page_radio_click_emits_typed_activation() {
        let first = action_control("First");
        let second = action_control("Second");
        for (control, selected) in [(&first, true), (&second, false)] {
            let mut element = page_radio(control, selected);
            let messages = click_messages(&mut element, control.id);
            assert_eq!(messages.len(), 1);
            assert!(matches!(
                &messages[0],
                ControlEvent { id: Some(id), input: ControlInput::Activate } if *id == control.id
            ));
        }
    }
}
