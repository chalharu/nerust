//! Owned settings presentation. No editor borrows or native widgets survive construction.
//!
//! Retain IDs in callbacks, then resolve against a fresh screen before binding:
//! a snapshot intentionally does not track later topology/enabled-state changes.

use nerust_gui_viewmodel::settings::{
    ValidationBanner,
    dto::{BindingValueView, CaptureStateView},
};
use nerust_input_traits::AttachmentId;
use nerust_settings_core::{editor::CaptureTarget, model::language::AppLanguage};
use nerust_system_traits::factory::descriptor::SystemSettingsFieldId;
use wakui::Key;

use super::{SettingsController, SettingsEvent, SettingsPage};

#[path = "screen_builder.rs"]
mod builder;

/// Canonical registry identity, not a localized label or a positional event key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SettingsSystemId(pub String);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotificationGroup {
    Info,
    Warning,
    Error,
    Display,
    Status,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SettingsTabId {
    System(SettingsSystemId),
    Input(SettingsSystemId),
    Shortcuts,
    Attachment {
        system: SettingsSystemId,
        attachment: String,
    },
    /// DTO sections without any binding targets have no attachment identity.
    EmptyAttachment {
        system: SettingsSystemId,
        index: usize,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SettingsTabsId {
    Systems,
    Inputs,
    Attachments(SettingsSystemId),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SettingsControlId {
    Page(SettingsPage),
    Tab(SettingsTabId),
    Language,
    StoragePolicy,
    StorageDirectory,
    StorageError,
    Fullscreen,
    Scaling,
    Vsync,
    Mute,
    Volume,
    SampleRate,
    Latency,
    InfoEnabled,
    InfoDuration,
    InfoColor,
    WarningEnabled,
    WarningDuration,
    WarningColor,
    ErrorEnabled,
    ErrorPresentation,
    ErrorColor,
    StatusEnabled,
    Position,
    StatusPosition,
    AreaMode,
    StatusAreaMode,
    StatusAreaFollows,
    SystemField {
        system: SettingsSystemId,
        field: SystemSettingsFieldId,
    },
    ControllerSlot {
        system: SettingsSystemId,
        slot: AttachmentId,
    },
    Binding(CaptureTarget),
    Conflict {
        system: SettingsSystemId,
        index: usize,
    },
    NoSystems,
    Cancel,
    Submit,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SettingsLayoutId {
    Page(SettingsPage),
    Row(SettingsControlId),
    Notification(NotificationGroup),
    Tab(SettingsTabId),
    Slots(SettingsSystemId),
    Conflicts(SettingsSystemId),
    Footer,
}

/// Visibility is intrinsic, not whether an ancestor page/tab is selected.
/// Visitors can retain hidden-page widgets; immediate-mode visitors use `selected`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SettingsMetadata {
    pub enabled: bool,
    pub visible: bool,
    pub accessible_label: String,
    pub help: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingsDisplay {
    Standard,
    Primary,
    Secondary,
    Note,
    Heading,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SettingsControlKind {
    Toggle {
        value: bool,
    },
    Choice {
        choices: Vec<String>,
        /// None means no matching choice; Some(index) may select a domain None.
        selected: Option<usize>,
    },
    Number {
        value: u32,
        min: u32,
        max: u32,
        /// Increment hint only, not a divisibility constraint.
        step: u32,
        unit: String,
        formatted: String,
    },
    Text {
        value: String,
        placeholder: String,
        /// Activate on this text control means browse; Text means edit.
        browse_label: Option<String>,
    },
    Binding {
        value: BindingValueView,
        change_label: String,
        clear_label: String,
    },
    Action,
    Note {
        text: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ControlInput {
    Bool(bool),
    Select(usize),
    Number(u32),
    Text(String),
    Activate,
    CaptureClear,
    CaptureKey(Key),
}

#[derive(Debug, Clone)]
enum ControlBinding {
    Toggle(fn(bool) -> SettingsEvent),
    Choice(Vec<SettingsEvent>),
    Number(fn(u32) -> SettingsEvent),
    Text(fn(String) -> SettingsEvent),
    Action(SettingsEvent),
    Capture(CaptureTarget),
    None,
}

#[derive(Debug, Clone)]
pub struct SettingsControl {
    pub id: SettingsControlId,
    pub label: String,
    pub metadata: SettingsMetadata,
    pub display: SettingsDisplay,
    pub kind: SettingsControlKind,
}

/// Domain binding table, kept separate from the pure view tree.
/// The screen owns both; controls never carry their own binding.
#[derive(Debug, Clone, Default)]
pub struct BindingTable(Vec<(SettingsControlId, ControlBinding, Vec<SettingsEvent>)>);

impl BindingTable {
    fn push(
        &mut self,
        id: SettingsControlId,
        binding: ControlBinding,
        context: Vec<SettingsEvent>,
    ) {
        self.0.push((id, binding, context));
    }

    /// Rejects wrong input kinds, unknown indices, invalid ranges, and disabled
    /// or intrinsically hidden controls. Apply the returned events in order.
    pub(crate) fn bind(
        &self,
        screen: &SettingsScreen,
        id: &SettingsControlId,
        input: ControlInput,
    ) -> Option<Vec<SettingsEvent>> {
        use ControlInput as I;
        use SettingsControlKind as K;
        let control = screen.control(id)?;
        if !control.metadata.enabled || !control.metadata.visible {
            return None;
        }
        let (_, binding, context) = self.0.iter().find(|(entry, _, _)| entry == id)?;
        let event = match (&control.kind, binding, input) {
            (K::Toggle { .. }, ControlBinding::Toggle(event), I::Bool(value)) => event(value),
            (K::Choice { choices, .. }, ControlBinding::Choice(events), I::Select(index)) => {
                choices.get(index)?;
                events.get(index)?.clone()
            }
            (K::Number { min, max, .. }, ControlBinding::Number(event), I::Number(value))
                if (*min..=*max).contains(&value) =>
            {
                event(value)
            }
            (K::Text { .. }, ControlBinding::Text(event), I::Text(value)) => event(value),
            (
                K::Text {
                    browse_label: Some(_),
                    ..
                },
                ControlBinding::Text(_),
                I::Activate,
            ) => SettingsEvent::BrowseStorageDirectory,
            (K::Action, ControlBinding::Action(event), I::Activate) => event.clone(),
            (K::Binding { .. }, ControlBinding::Capture(target), I::Activate) => {
                SettingsEvent::StartCapture(target.clone())
            }
            (K::Binding { .. }, ControlBinding::Capture(target), I::CaptureClear) => {
                SettingsEvent::ClearCapture(target.clone())
            }
            (
                K::Binding {
                    value: BindingValueView::Capturing(_),
                    ..
                },
                ControlBinding::Capture(_),
                I::CaptureKey(key),
            ) => SettingsEvent::CaptureKey(key),
            _ => return None,
        };
        let mut events = context.clone();
        events.push(event);
        Some(events)
    }
}

#[derive(Debug, Clone)]
pub struct SettingsLayout {
    pub id: SettingsLayoutId,
    pub label: String,
    pub metadata: SettingsMetadata,
    pub spacing: u16,
    pub indent: u16,
    /// Group header action (e.g. notification enable), visited before children.
    pub header: Option<SettingsControl>,
    pub children: Vec<SettingsNode>,
}

#[derive(Debug, Clone)]
pub enum SettingsNode {
    Row(SettingsLayout),
    Column(SettingsLayout),
    Group(SettingsLayout),
    Tabs(SettingsTabs),
    Control(SettingsControl),
}

#[derive(Debug, Clone)]
pub struct SettingsTabs {
    pub id: SettingsTabsId,
    pub metadata: SettingsMetadata,
    pub selected: Option<usize>,
    pub tabs: Vec<SettingsTab>,
}

#[derive(Debug, Clone)]
pub struct SettingsTab {
    pub id: SettingsTabId,
    pub label: String,
    pub selected: bool,
    pub select: SettingsControl,
    pub body: SettingsLayout,
}

#[derive(Debug, Clone)]
pub struct SettingsScreenPage {
    pub id: SettingsPage,
    pub label: String,
    pub selected: bool,
    pub select: SettingsControl,
    pub body: SettingsLayout,
}

#[derive(Debug, Clone)]
pub struct SettingsScreen {
    pub title: String,
    pub language: AppLanguage,
    pub pages: Vec<SettingsScreenPage>,
    pub validation: ValidationBanner,
    pub error_message: Option<String>,
    /// Ready-to-render native error, otherwise validation headline.
    pub message: Option<String>,
    pub capture: CaptureStateView,
    pub footer: SettingsLayout,
    /// Domain bindings, separate from the pure view tree above.
    pub bindings: BindingTable,
}

impl SettingsController {
    pub fn screen(&self) -> SettingsScreen {
        builder::build(self)
    }
}

impl SettingsScreen {
    /// Includes every page/tab regardless of selection. Begin/end calls always
    /// balance. Navigation controls are supplied on page/tab, not emitted twice.
    pub fn visit(&self, visitor: &mut dyn SettingsVisitor) {
        visitor.begin_screen(self);
        for page in &self.pages {
            visitor.begin_page(page);
            visit_layout(&page.body, LayoutKind::Column, visitor);
            visitor.end_page(page);
        }
        visit_layout(&self.footer, LayoutKind::Row, visitor);
        visitor.end_screen(self);
    }

    /// Resolve retained native IDs against current values before handling input.
    pub fn control(&self, id: &SettingsControlId) -> Option<&SettingsControl> {
        for page in &self.pages {
            if &page.select.id == id {
                return Some(&page.select);
            }
            if let Some(control) = find_layout(&page.body, id) {
                return Some(control);
            }
        }
        find_layout(&self.footer, id)
    }

    /// Native keyboard adapters need no knowledge of the selected input page.
    pub fn bind_capture_key(&self, key: Key) -> Option<Vec<SettingsEvent>> {
        self.capture.target.as_ref()?;
        Some(vec![SettingsEvent::CaptureKey(key)])
    }
}

impl wakui::binding::ViewDefinition for SettingsScreen {
    type ViewId = SettingsControlId;
    type ViewInput = ControlInput;
    type DomainEvent = SettingsEvent;

    /// Total 1:1 variant mapping; domain validity stays in `bind`.
    fn convert(input: wakui::control::ControlInput) -> Option<ControlInput> {
        use wakui::control::ControlInput as I;
        Some(match input {
            I::Bool(v) => ControlInput::Bool(v),
            I::Select(v) => ControlInput::Select(v),
            I::Number(v) => ControlInput::Number(v),
            I::Text(v) => ControlInput::Text(v),
            I::Activate => ControlInput::Activate,
            I::CaptureClear => ControlInput::CaptureClear,
            I::CaptureKey(v) => ControlInput::CaptureKey(v),
        })
    }

    /// Resolve retained native IDs against current values before handling input.
    fn bind(&self, id: &SettingsControlId, input: ControlInput) -> Option<Vec<SettingsEvent>> {
        self.bindings.bind(self, id, input)
    }
}

fn find_layout<'a>(
    layout: &'a SettingsLayout,
    id: &SettingsControlId,
) -> Option<&'a SettingsControl> {
    if let Some(header) = &layout.header
        && &header.id == id
    {
        return Some(header);
    }
    layout.children.iter().find_map(|node| match node {
        SettingsNode::Control(control) => (&control.id == id).then_some(control),
        SettingsNode::Row(layout) | SettingsNode::Column(layout) | SettingsNode::Group(layout) => {
            find_layout(layout, id)
        }
        SettingsNode::Tabs(tabs) => tabs.tabs.iter().find_map(|tab| {
            if &tab.select.id == id {
                Some(&tab.select)
            } else {
                find_layout(&tab.body, id)
            }
        }),
    })
}

/// Object-safe and intentionally unconstrained by Send or a native Widget type.
/// All arguments refer only to the owned snapshot, so callbacks may reenter the
/// controller after its construction borrow has been released.
pub trait SettingsVisitor {
    fn begin_screen(&mut self, _screen: &SettingsScreen) {}
    fn end_screen(&mut self, _screen: &SettingsScreen) {}
    fn begin_page(&mut self, _page: &SettingsScreenPage) {}
    fn end_page(&mut self, _page: &SettingsScreenPage) {}
    fn begin_row(&mut self, _layout: &SettingsLayout) {}
    fn end_row(&mut self, _layout: &SettingsLayout) {}
    fn begin_column(&mut self, _layout: &SettingsLayout) {}
    fn end_column(&mut self, _layout: &SettingsLayout) {}
    fn begin_group(&mut self, _layout: &SettingsLayout) {}
    fn end_group(&mut self, _layout: &SettingsLayout) {}
    fn begin_tabs(&mut self, _tabs: &SettingsTabs) {}
    fn end_tabs(&mut self, _tabs: &SettingsTabs) {}
    fn begin_tab(&mut self, _tab: &SettingsTab) {}
    fn end_tab(&mut self, _tab: &SettingsTab) {}
    fn control(&mut self, _control: &SettingsControl) {}
}

#[derive(Clone, Copy)]
enum LayoutKind {
    Row,
    Column,
    Group,
}

fn visit_layout(layout: &SettingsLayout, kind: LayoutKind, visitor: &mut dyn SettingsVisitor) {
    match kind {
        LayoutKind::Row => visitor.begin_row(layout),
        LayoutKind::Column => visitor.begin_column(layout),
        LayoutKind::Group => visitor.begin_group(layout),
    }
    if let Some(header) = &layout.header {
        visitor.control(header);
    }
    for node in &layout.children {
        match node {
            SettingsNode::Row(layout) => visit_layout(layout, LayoutKind::Row, visitor),
            SettingsNode::Column(layout) => visit_layout(layout, LayoutKind::Column, visitor),
            SettingsNode::Group(layout) => visit_layout(layout, LayoutKind::Group, visitor),
            SettingsNode::Control(control) => visitor.control(control),
            SettingsNode::Tabs(tabs) => {
                visitor.begin_tabs(tabs);
                for tab in &tabs.tabs {
                    visitor.begin_tab(tab);
                    visit_layout(&tab.body, LayoutKind::Column, visitor);
                    visitor.end_tab(tab);
                }
                visitor.end_tabs(tabs);
            }
        }
    }
    match kind {
        LayoutKind::Row => visitor.end_row(layout),
        LayoutKind::Column => visitor.end_column(layout),
        LayoutKind::Group => visitor.end_group(layout),
    }
}

#[cfg(test)]
#[path = "screen_tests.rs"]
mod tests;
