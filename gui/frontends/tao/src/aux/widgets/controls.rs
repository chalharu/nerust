//! Domain-free immediate-mode interpreter of host-owned control scenes.

use iced::{
    Alignment, Length,
    widget::{
        button, checkbox, column, container, pick_list, row, scrollable, slider, text, text_input,
    },
};
use nerust_frontend_host::control::{
    Control, ControlDisplay, ControlInput, ControlKind, ControlMetadata, ControlScreen,
    ControlVisitor, Layout, LayoutKind, Page, Tab, Tabs,
};

use super::bindings::{El, Message};

/// All labels and callback IDs are owned; the snapshot may be dropped.
#[cfg(test)]
pub(super) fn build(window: &nerust_frontend_host::window::Window) -> El<'static> {
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
impl nerust_frontend_host::window::WindowVisitor for ControlsWindowVisitor {
    fn visit_controls(&mut self, screen: &ControlScreen) {
        self.output = Some(build_screen(screen));
    }
}

pub(crate) fn build_screen(screen: &ControlScreen) -> El<'static> {
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

    fn finish_layout(&mut self, layout: &Layout, kind: LayoutKind) {
        let mut frame = self.stack.pop().expect("layout frame");
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
        let element = if matches!(kind, LayoutKind::Row) {
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
        } else if matches!(kind, LayoutKind::Group) {
            let mut header = row![].spacing(spacing).align_y(Alignment::Center);
            if !layout.label.is_empty() {
                header = header.push(text(layout.label.clone()).font(iced::Font {
                    weight: iced::font::Weight::Bold,
                    ..self.font
                }));
            }
            if layout.header.as_ref().is_some_and(|c| c.metadata.visible)
                && !frame.children.is_empty()
            {
                header = header.push(frame.children.remove(0));
            }
            column![
                header,
                column(frame.children)
                    .spacing(spacing)
                    .padding(iced::padding::left(f32::from(layout.indent)))
            ]
            .spacing(spacing)
            .into()
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

fn table_element(children: Vec<TableChild>, layout: &Layout, font: iced::Font) -> El<'static> {
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
        let mut root = column![
            text(screen.title.clone()).font(self.font).size(20),
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
        ]
        .spacing(12)
        .padding(16)
        .height(Length::Fill);
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
            self.sidebar
                .push(navigation(&page.select, page.selected, true));
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

    fn begin_layout(&mut self, layout: &Layout, kind: LayoutKind) {
        self.push_metadata(&layout.metadata);
        if matches!(kind, LayoutKind::Table) {
            self.stack.last_mut().expect("table frame").table = Some(Vec::new());
        }
    }
    fn end_layout(&mut self, layout: &Layout, kind: LayoutKind) {
        self.finish_layout(layout, kind);
    }

    fn begin_tabs(&mut self, tabs: &Tabs) {
        self.push_metadata(&tabs.metadata);
    }
    fn end_tabs(&mut self, _: &Tabs) {
        let frame = self.stack.pop().expect("tabs frame");
        if frame.visible {
            self.append(
                column![row(frame.navigation).spacing(4), column(frame.children)]
                    .spacing(12)
                    .into(),
            );
        }
    }
    fn begin_tab(&mut self, tab: &Tab) {
        let parent = self.stack.last_mut().expect("tabs frame");
        if parent.visible && tab.select.metadata.visible {
            parent
                .navigation
                .push(navigation(&tab.select, tab.selected, parent.enabled));
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
        }
    }
}

fn navigation(control: &Control, selected: bool, enabled: bool) -> El<'static> {
    button(text(control.label.clone()))
        .style(if selected {
            button::primary
        } else {
            button::secondary
        })
        .on_press_maybe(
            (enabled && control.metadata.enabled && !selected)
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

fn control_element(control: &Control, parent_enabled: bool, font: iced::Font) -> El<'static> {
    let enabled = parent_enabled && control.metadata.enabled;
    let id = control.id;
    let input = move |value| Message::new(id, value);
    let label = || {
        text(control.label.clone())
            .font(font)
            .width(Length::Fixed(220.0))
    };
    let element: El<'static> = match &control.kind {
        ControlKind::Toggle { value } => checkbox(*value)
            .label(control.label.clone())
            .on_toggle_maybe(enabled.then_some(move |v| input(ControlInput::Bool(v))))
            .into(),
        ControlKind::Choice { choices, selected } => {
            let options: Vec<_> = choices
                .iter()
                .enumerate()
                .map(|(index, label)| Choice {
                    index,
                    label: label.clone(),
                })
                .collect();
            let selected = selected.and_then(|i| options.get(i)).cloned();
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
            row![label(), super::disabled::gate(pick.into(), enabled)]
                .spacing(12)
                .into()
        }
        ControlKind::Number {
            value,
            min,
            max,
            step,
            formatted,
            ..
        } => {
            let native = slider(*min..=*max, *value, move |v| input(ControlInput::Number(v)))
                .step(*step)
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
                label(),
                super::disabled::gate(native.into(), enabled),
                text(formatted.clone()).font(font)
            ]
            .spacing(12)
            .align_y(Alignment::Center)
            .into()
        }
        ControlKind::Text {
            value,
            placeholder,
            browse_label,
        } => {
            let edit_id = control.id;
            let edit = text_input(placeholder, value)
                .font(font)
                .width(Length::Fill)
                .on_input_maybe(
                    enabled.then_some(move |v| Message::new(edit_id, ControlInput::Text(v))),
                );
            let mut items = row![label(), super::disabled::gate(edit.into(), enabled)]
                .spacing(12)
                .align_y(Alignment::Center);
            if let Some(browse) = browse_label {
                items = items.push(
                    button(text(browse.clone()).font(font))
                        .on_press_maybe(enabled.then(|| input(ControlInput::Activate))),
                );
            }
            items.into()
        }
        ControlKind::Binding {
            text: value,
            change_label,
            clear_label,
            ..
        } => row![
            label(),
            text(value.clone()).font(font).width(Length::Fill),
            button(text(change_label.clone()).font(font))
                .on_press_maybe(enabled.then(|| input(ControlInput::Activate))),
            button(text(clear_label.clone()).font(font))
                .on_press_maybe(enabled.then(|| input(ControlInput::CaptureClear)))
        ]
        .spacing(12)
        .align_y(Alignment::Center)
        .into(),
        ControlKind::Action => button(text(control.label.clone()).font(font))
            .style(if control.display == ControlDisplay::Primary {
                button::primary
            } else {
                button::secondary
            })
            .on_press_maybe(enabled.then(|| input(ControlInput::Activate)))
            .into(),
        ControlKind::Note { text: value } => {
            let note = text(value.clone()).font(font).size(
                if control.display == ControlDisplay::Heading {
                    16
                } else {
                    13
                },
            );
            if control.label.is_empty() {
                note.into()
            } else {
                row![label(), note].spacing(12).into()
            }
        }
    };
    if let Some(help) = &control.metadata.help {
        column![element, text(help.clone()).font(font).size(13)]
            .spacing(4)
            .into()
    } else {
        element
    }
}

#[cfg(test)]
mod tests;
