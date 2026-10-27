//! Generic immediate-mode interpreter of the host-owned settings AST.

use iced::{
    Alignment, Length,
    widget::{
        button, checkbox, column, container, pick_list, row, scrollable, slider, text, text_input,
    },
};
use nerust_frontend_host::settings::{
    ControlInput, SettingsControl, SettingsControlKind, SettingsDisplay, SettingsLayout,
    SettingsMetadata, SettingsScreen, SettingsScreenPage, SettingsTab, SettingsTabs,
    SettingsVisitor,
};
use nerust_gui_viewmodel::settings::dto::BindingValueView;

use super::bindings::{El, Message};

/// All labels and callback IDs are owned; the snapshot may be dropped.
#[cfg(test)]
pub(super) fn build(window: &nerust_frontend_host::window::Window) -> El<'static> {
    let mut visitor = SettingsWindowVisitor::default();
    window.visit(&mut visitor);
    visitor
        .output
        .expect("settings window must contain a real settings screen")
}

#[cfg(test)]
#[derive(Default)]
struct SettingsWindowVisitor {
    output: Option<El<'static>>,
}

#[cfg(test)]
impl nerust_frontend_host::window::WindowVisitor for SettingsWindowVisitor {
    fn visit_settings(&mut self, screen: &SettingsScreen) {
        self.output = Some(build_screen(screen));
    }
}

pub(crate) fn build_screen(screen: &SettingsScreen) -> El<'static> {
    let mut visitor = IcedSettingsVisitor::default();
    screen.visit(&mut visitor as &mut dyn SettingsVisitor);
    visitor.output.expect("settings traversal must finish")
}

#[derive(Default)]
struct Frame {
    visible: bool,
    enabled: bool,
    children: Vec<El<'static>>,
    navigation: Vec<El<'static>>,
}

#[derive(Default)]
struct IcedSettingsVisitor {
    stack: Vec<Frame>,
    sidebar: Vec<El<'static>>,
    page: Option<El<'static>>,
    output: Option<El<'static>>,
    font: iced::Font,
}

impl IcedSettingsVisitor {
    fn push(&mut self, visible: bool, enabled: bool) {
        let parent = self.stack.last();
        self.stack.push(Frame {
            visible: visible && parent.is_none_or(|f| f.visible),
            enabled: enabled && parent.is_none_or(|f| f.enabled),
            ..Frame::default()
        });
    }

    fn push_metadata(&mut self, metadata: &SettingsMetadata) {
        self.push(metadata.visible, metadata.enabled);
    }

    fn append(&mut self, element: El<'static>) {
        let frame = self.stack.last_mut().expect("settings frame");
        if frame.visible {
            frame.children.push(element);
        }
    }

    fn finish_layout(&mut self, layout: &SettingsLayout, horizontal: bool, group: bool) {
        let mut frame = self.stack.pop().expect("layout frame");
        if !frame.visible || (frame.children.is_empty() && layout.label.is_empty()) {
            return;
        }
        let spacing = f32::from(layout.spacing);
        let element = if horizontal {
            if !layout.label.is_empty() {
                frame
                    .children
                    .insert(0, text(layout.label.clone()).font(self.font).into());
            }
            row(frame.children)
                .spacing(spacing)
                .align_y(Alignment::Center)
                .padding(iced::padding::left(f32::from(layout.indent)))
                .into()
        } else if group {
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

impl SettingsVisitor for IcedSettingsVisitor {
    fn begin_screen(&mut self, screen: &SettingsScreen) {
        self.font = nerust_settings_core::i18n::font_family(screen.language)
            .map(iced::Font::with_name)
            .unwrap_or_default();
        self.push(true, true);
    }

    fn end_screen(&mut self, screen: &SettingsScreen) {
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

    fn begin_page(&mut self, page: &SettingsScreenPage) {
        if page.select.metadata.visible {
            self.sidebar
                .push(navigation(&page.select, page.selected, true));
        }
        self.push(page.selected && page.select.metadata.visible, true);
    }

    fn end_page(&mut self, _: &SettingsScreenPage) {
        let frame = self.stack.pop().expect("page frame");
        if frame.visible {
            self.page = Some(column(frame.children).into());
        }
    }

    fn begin_row(&mut self, layout: &SettingsLayout) {
        self.push_metadata(&layout.metadata);
    }
    fn end_row(&mut self, layout: &SettingsLayout) {
        self.finish_layout(layout, true, false);
    }
    fn begin_column(&mut self, layout: &SettingsLayout) {
        self.push_metadata(&layout.metadata);
    }
    fn end_column(&mut self, layout: &SettingsLayout) {
        self.finish_layout(layout, false, false);
    }
    fn begin_group(&mut self, layout: &SettingsLayout) {
        self.push_metadata(&layout.metadata);
    }
    fn end_group(&mut self, layout: &SettingsLayout) {
        self.finish_layout(layout, false, true);
    }

    fn begin_tabs(&mut self, tabs: &SettingsTabs) {
        self.push_metadata(&tabs.metadata);
    }
    fn end_tabs(&mut self, _: &SettingsTabs) {
        let frame = self.stack.pop().expect("tabs frame");
        if frame.visible {
            self.append(
                column![row(frame.navigation).spacing(4), column(frame.children)]
                    .spacing(12)
                    .into(),
            );
        }
    }
    fn begin_tab(&mut self, tab: &SettingsTab) {
        let parent = self.stack.last_mut().expect("tabs frame");
        if parent.visible && tab.select.metadata.visible {
            parent
                .navigation
                .push(navigation(&tab.select, tab.selected, parent.enabled));
        }
        self.push(tab.selected && tab.select.metadata.visible, true);
    }
    fn end_tab(&mut self, _: &SettingsTab) {
        let frame = self.stack.pop().expect("tab frame");
        if frame.visible {
            self.append(column(frame.children).into());
        }
    }
    fn control(&mut self, control: &SettingsControl) {
        let frame = self.stack.last().expect("control frame");
        if frame.visible && control.metadata.visible {
            self.append(control_element(control, frame.enabled, self.font));
        }
    }
}

fn navigation(control: &SettingsControl, selected: bool, enabled: bool) -> El<'static> {
    button(text(control.label.clone()))
        .style(if selected {
            button::primary
        } else {
            button::secondary
        })
        .on_press_maybe(
            (enabled && control.metadata.enabled && !selected)
                .then(|| Message::Input(control.id.clone(), ControlInput::Activate)),
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

fn control_element(
    control: &SettingsControl,
    parent_enabled: bool,
    font: iced::Font,
) -> El<'static> {
    let enabled = parent_enabled && control.metadata.enabled;
    let id = control.id.clone();
    let input = move |value| Message::Input(id.clone(), value);
    let label = || {
        text(control.label.clone())
            .font(font)
            .width(Length::Fixed(220.0))
    };
    let element: El<'static> = match &control.kind {
        SettingsControlKind::Toggle { value } => checkbox(*value)
            .label(control.label.clone())
            .on_toggle_maybe(enabled.then_some(move |v| input(ControlInput::Bool(v))))
            .into(),
        SettingsControlKind::Choice { choices, selected } => {
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
        SettingsControlKind::Number {
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
        SettingsControlKind::Text {
            value,
            placeholder,
            browse_label,
        } => {
            let edit_id = control.id.clone();
            let edit = text_input(placeholder, value)
                .font(font)
                .width(Length::Fill)
                .on_input_maybe(
                    enabled
                        .then_some(move |v| Message::Input(edit_id.clone(), ControlInput::Text(v))),
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
        SettingsControlKind::Binding {
            value,
            change_label,
            clear_label,
        } => {
            let (BindingValueView::Bound(value)
            | BindingValueView::Unbound(value)
            | BindingValueView::Capturing(value)) = value;
            row![
                label(),
                text(value.clone()).font(font).width(Length::Fill),
                button(text(change_label.clone()).font(font))
                    .on_press_maybe(enabled.then(|| input(ControlInput::Activate))),
                button(text(clear_label.clone()).font(font))
                    .on_press_maybe(enabled.then(|| input(ControlInput::CaptureClear)))
            ]
            .spacing(12)
            .align_y(Alignment::Center)
            .into()
        }
        SettingsControlKind::Action => button(text(control.label.clone()).font(font))
            .style(if control.display == SettingsDisplay::Primary {
                button::primary
            } else {
                button::secondary
            })
            .on_press_maybe(enabled.then(|| input(ControlInput::Activate)))
            .into(),
        SettingsControlKind::Note { text: value } => {
            let note = text(value.clone()).font(font).size(13);
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
