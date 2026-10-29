//! Owned settings dialog state, projected directly as a generic [`ControlScreen`].
//!
//! There is no intermediate settings widget tree: the builder emits
//! framework controls with stable [`ControlId`]s minted from the domain key
//! enums below, and [`BindingTable`] maps those IDs back to domain events.
//! Native adapters only see the projected screen.

use nerust_input_traits::AttachmentId;
use nerust_settings_core::{editor::CaptureTarget, model::language::AppLanguage};
use nerust_system_traits::factory::descriptor::SystemSettingsFieldId;
use wakui::Key;
use wakui::scene::control::{
    Capture, ColumnPolicy, Container, Control, ControlDisplay, ControlId, ControlInput,
    ControlMetadata, ControlScreen, GroupPolicy, IdRegistry, LayoutPolicy, Node, Page, RowPolicy,
    Tabs,
};
use wakui::scene::leaf::{
    ActionLeaf, BindingLeaf, ChoiceLeaf, ControlLeaf, NoteLeaf, NumberLeaf, TextLeaf, ToggleLeaf,
};

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

/// Stable domain key for a bindable settings control.
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

/// Stable domain key for a settings container.
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

/// Internal stable key unifying every minted ID, including content-addressed
/// legacy rows that lack domain identity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum StableKey {
    Control(SettingsControlId),
    Layout(SettingsLayoutId),
    Tabs(SettingsTabsId),
    Tab(SettingsTabId),
    Page(SettingsPage),
    EmptyControl(ContentKey),
    EmptyLayout(ContentKey),
    EmptyTab(ContentKey),
    Conflict(ContentKey),
    ConflictRow(ContentKey),
}

pub(crate) type StableKeyRegistry = IdRegistry<StableKey>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ContentKey {
    pub system: SettingsSystemId,
    pub label: String,
    pub occurrence: usize,
}

#[derive(Debug, Clone)]
pub(crate) enum ControlBinding {
    Toggle(fn(bool) -> SettingsEvent),
    Choice(Vec<SettingsEvent>),
    Number(fn(u32) -> SettingsEvent),
    Text(fn(String) -> SettingsEvent),
    Action(SettingsEvent),
    Capture(CaptureTarget),
    None,
}

/// Domain binding table, keyed by minted [`ControlId`].
#[derive(Debug, Clone, Default)]
pub struct BindingTable(Vec<(ControlId, ControlBinding, Vec<SettingsEvent>)>);

impl BindingTable {
    fn push(&mut self, id: ControlId, binding: ControlBinding, context: Vec<SettingsEvent>) {
        self.0.push((id, binding, context));
    }

    /// Rejects wrong input kinds, unknown indices, invalid ranges, and disabled
    /// or hidden controls. Apply the returned events in order.
    pub(crate) fn bind(
        &self,
        screen: &ControlScreen,
        id: &ControlId,
        input: ControlInput,
    ) -> Option<Vec<SettingsEvent>> {
        use wakui::scene::control::ControlInput as I;
        let control = screen.control(*id)?;
        if !control.metadata.enabled || !control.metadata.visible {
            return None;
        }
        let (_, binding, context) = self.0.iter().find(|(entry, _, _)| entry == id)?;
        let event = match (binding, input) {
            (ControlBinding::Toggle(event), I::Bool(value)) => {
                control.downcast::<ToggleLeaf>()?;
                event(value)
            }
            (ControlBinding::Choice(events), I::Select(index)) => {
                let leaf = control.downcast::<ChoiceLeaf>()?;
                leaf.choices.get(index)?;
                events.get(index)?.clone()
            }
            (ControlBinding::Number(event), I::Number(value)) => {
                let leaf = control.downcast::<NumberLeaf>()?;
                if !(leaf.min..=leaf.max).contains(&value) {
                    return None;
                }
                event(value)
            }
            (ControlBinding::Text(event), I::Text(value)) => {
                control.downcast::<TextLeaf>()?;
                event(value)
            }
            (ControlBinding::Text(_), I::Activate) => {
                let leaf = control.downcast::<TextLeaf>()?;
                leaf.browse_label.as_ref()?;
                SettingsEvent::BrowseStorageDirectory
            }
            (ControlBinding::Action(event), I::Activate) => {
                control.downcast::<ActionLeaf>()?;
                event.clone()
            }
            (ControlBinding::Capture(target), I::Activate) => {
                control.downcast::<BindingLeaf>()?;
                SettingsEvent::StartCapture(target.clone())
            }
            (ControlBinding::Capture(target), I::CaptureClear) => {
                control.downcast::<BindingLeaf>()?;
                SettingsEvent::ClearCapture(target.clone())
            }
            (ControlBinding::Capture(_), I::CaptureKey(key)) => {
                let leaf = control.downcast::<BindingLeaf>()?;
                if !leaf.capturing {
                    return None;
                }
                SettingsEvent::CaptureKey(key)
            }
            _ => return None,
        };
        let mut events = context.clone();
        events.push(event);
        Some(events)
    }
}

/// Projected settings screen: the generic control tree plus its binding table.
pub struct SettingsScreen {
    pub screen: ControlScreen,
    bindings: BindingTable,
}

impl SettingsController {
    pub fn screen(&self) -> SettingsScreen {
        builder::build(self, &mut self.ids.borrow_mut())
    }
}

impl SettingsScreen {
    /// Resolve a retained native ID against current values before handling input.
    pub fn control(&self, id: ControlId) -> Option<&Control> {
        self.screen.control(id)
    }

    /// Resolve retained native IDs against current values before handling input.
    pub fn bind(&self, id: &ControlId, input: ControlInput) -> Option<Vec<SettingsEvent>> {
        self.bindings.bind(&self.screen, id, input)
    }

    /// Native keyboard adapters need no knowledge of the selected input page.
    pub fn bind_capture_key(&self, key: Key) -> Option<Vec<SettingsEvent>> {
        self.screen.capture.target?;
        Some(vec![SettingsEvent::CaptureKey(key)])
    }
}

impl wakui::app::binding::ViewDefinition for SettingsScreen {
    type ViewId = ControlId;
    type ViewInput = ControlInput;
    type DomainEvent = SettingsEvent;

    /// Settings has no raster images; hover never binds here.
    fn convert(input: wakui::scene::control::ControlInput) -> Option<ControlInput> {
        match input {
            wakui::scene::control::ControlInput::Hover { .. } => None,
            other => Some(other),
        }
    }

    /// Resolve retained native IDs against current values before handling input.
    fn bind(&self, id: &ControlId, input: ControlInput) -> Option<Vec<SettingsEvent>> {
        self.bind(id, input)
    }
}

/// Screen builder: mints stable [`ControlId`]s from domain keys and fills the
/// binding table. View definitions stay declarative: this struct only
/// assembles framework controls, never walks or diffs them.
pub(crate) struct Builder<'a> {
    pub ids: &'a mut IdRegistry<StableKey>,
    pub table: BindingTable,
    empty_keys: Vec<(SettingsTabId, ContentKey)>,
    conflict_keys: Vec<(SettingsControlId, ContentKey)>,
}

impl<'a> Builder<'a> {
    pub(crate) fn from_registry(ids: &'a mut IdRegistry<StableKey>) -> Self {
        Self {
            ids,
            table: BindingTable::default(),
            empty_keys: Vec::new(),
            conflict_keys: Vec::new(),
        }
    }

    fn empty_key(
        &mut self,
        id: &SettingsTabId,
        system: &SettingsSystemId,
        label: &str,
    ) -> ContentKey {
        if let Some((_, key)) = self.empty_keys.iter().find(|(domain, _)| domain == id) {
            return key.clone();
        }
        let occurrence = self
            .empty_keys
            .iter()
            .filter(|(_, key)| key.system == *system && key.label == label)
            .count();
        let key = ContentKey {
            system: system.clone(),
            label: label.to_string(),
            occurrence,
        };
        self.empty_keys.push((id.clone(), key.clone()));
        key
    }

    fn conflict_key(
        &mut self,
        id: &SettingsControlId,
        system: &SettingsSystemId,
        label: &str,
    ) -> ContentKey {
        if let Some((_, key)) = self.conflict_keys.iter().find(|(domain, _)| domain == id) {
            return key.clone();
        }
        let occurrence = self
            .conflict_keys
            .iter()
            .filter(|(_, key)| key.system == *system && key.label == label)
            .count();
        let key = ContentKey {
            system: system.clone(),
            label: label.to_string(),
            occurrence,
        };
        self.conflict_keys.push((id.clone(), key.clone()));
        key
    }

    fn control_key(&mut self, id: &SettingsControlId, label: &str) -> StableKey {
        match id {
            SettingsControlId::Tab(tab @ SettingsTabId::EmptyAttachment { .. }) => {
                let system = match tab {
                    SettingsTabId::EmptyAttachment { system, .. } => system.clone(),
                    _ => unreachable!(),
                };
                StableKey::EmptyControl(self.empty_key(tab, &system, label))
            }
            conflict @ SettingsControlId::Conflict { system, .. } => {
                StableKey::Conflict(self.conflict_key(conflict, system, label))
            }
            _ => StableKey::Control(id.clone()),
        }
    }

    /// Mint a page control (navigation) with its binding registered.
    pub(crate) fn page_select(&mut self, id: SettingsPage, label: String) -> Control {
        let token = self.ids.id(StableKey::Control(SettingsControlId::Page(id)));
        self.table.push(
            token,
            ControlBinding::Action(SettingsEvent::SelectPage(id)),
            Vec::new(),
        );
        Control {
            id: token,
            label: label.clone(),
            metadata: ControlMetadata::new(label),
            display: ControlDisplay::Standard,
            leaf: Box::new(ActionLeaf),
        }
    }

    fn fresh_container(
        &mut self,
        key: StableKey,
        header: Option<Control>,
        policy: Box<dyn LayoutPolicy>,
        children: Vec<Node>,
        spacing: f64,
        indent: f64,
    ) -> Container {
        let mut metadata = ControlMetadata::new("");
        metadata.hints.spacing = Some(spacing);
        metadata.hints.indent = Some(indent);
        Container {
            id: self.ids.id(key),
            label: String::new(),
            metadata,
            header,
            policy,
            children,
        }
    }

    pub(crate) fn page_body(&mut self, id: SettingsPage, children: Vec<Node>) -> Container {
        self.fresh_container(
            StableKey::Layout(SettingsLayoutId::Page(id)),
            None,
            Box::new(ColumnPolicy),
            children,
            12.0,
            0.0,
        )
    }

    pub(crate) fn footer(&mut self, children: Vec<Node>) -> Container {
        self.fresh_container(
            StableKey::Layout(SettingsLayoutId::Footer),
            None,
            Box::new(RowPolicy),
            children,
            12.0,
            0.0,
        )
    }

    pub(crate) fn row(&mut self, control: Control, control_key: &SettingsControlId) -> Node {
        let layout = self.fresh_container(
            StableKey::Layout(SettingsLayoutId::Row(control_key.clone())),
            None,
            Box::new(RowPolicy),
            vec![Node::Control(control)],
            12.0,
            0.0,
        );
        Node::Container(Box::new(layout))
    }

    pub(crate) fn conflict_row(
        &mut self,
        control: Control,
        id: &SettingsControlId,
        label: &str,
    ) -> Node {
        let system = match id {
            SettingsControlId::Conflict { system, .. } => system.clone(),
            _ => unreachable!(),
        };
        let row_key = self.conflict_key(id, &system, label);
        let layout = self.fresh_container(
            StableKey::ConflictRow(row_key),
            None,
            Box::new(RowPolicy),
            vec![Node::Control(control)],
            12.0,
            0.0,
        );
        Node::Container(Box::new(layout))
    }

    pub(crate) fn group(
        &mut self,
        language: AppLanguage,
        group: NotificationGroup,
        title: String,
        header: Option<Control>,
        children: Vec<Node>,
        indent: f64,
    ) -> Node {
        use nerust_settings_core::i18n::{notification_group_label, text};
        let mut metadata = ControlMetadata::new(title.clone());
        metadata.hints.spacing = Some(8.0);
        metadata.hints.indent = Some(indent);
        let layout = Container {
            id: self
                .ids
                .id(StableKey::Layout(SettingsLayoutId::Notification(group))),
            label: title.clone(),
            metadata,
            header: header.map(|mut header| {
                header.metadata.accessible_label = if group == NotificationGroup::Status {
                    text(
                        language,
                        nerust_settings_core::i18n::UiText::NotificationStatus,
                    )
                    .into()
                } else {
                    notification_group_label(language, &title)
                };
                header
            }),
            policy: Box::new(GroupPolicy),
            children,
        };
        Node::Container(Box::new(layout))
    }

    pub(crate) fn column(&mut self, key: SettingsLayoutId, children: Vec<Node>) -> Node {
        Node::Container(Box::new(self.fresh_container(
            StableKey::Layout(key),
            None,
            Box::new(ColumnPolicy),
            children,
            12.0,
            0.0,
        )))
    }

    pub(crate) fn tabs(
        &mut self,
        tabs_id: SettingsTabsId,
        selected: Option<usize>,
        tabs: Vec<TabSpec>,
    ) -> Node {
        let built: Vec<wakui::scene::control::Tab> = tabs
            .into_iter()
            .map(
                |TabSpec {
                     id,
                     label,
                     selected: tab_selected,
                     context,
                     children,
                 }| {
                    let is_empty = matches!(id, SettingsTabId::EmptyAttachment { .. });
                    let empty_content = if is_empty {
                        match &id {
                            SettingsTabId::EmptyAttachment { system, .. } => {
                                Some(self.empty_key(&id, system, &label))
                            }
                            _ => unreachable!(),
                        }
                    } else {
                        None
                    };
                    let body_key = match &empty_content {
                        Some(key) => StableKey::EmptyLayout(key.clone()),
                        None => StableKey::Layout(SettingsLayoutId::Tab(id.clone())),
                    };
                    let body = self.fresh_container(
                        body_key,
                        None,
                        Box::new(ColumnPolicy),
                        children,
                        12.0,
                        0.0,
                    );
                    let mut select_context = context.clone();
                    let event = select_context.pop().expect("tab select event");
                    let token = match &empty_content {
                        Some(key) => self.ids.id(StableKey::EmptyControl(key.clone())),
                        None => self
                            .ids
                            .id(StableKey::Control(SettingsControlId::Tab(id.clone()))),
                    };
                    self.table
                        .push(token, ControlBinding::Action(event), select_context);
                    wakui::scene::control::Tab {
                        id: self.ids.id(match &empty_content {
                            Some(key) => StableKey::EmptyTab(key.clone()),
                            None => StableKey::Tab(id.clone()),
                        }),
                        label: label.clone(),
                        selected: tab_selected,
                        select: Control {
                            id: token,
                            label: label.clone(),
                            metadata: ControlMetadata::new(label),
                            display: ControlDisplay::Standard,
                            leaf: Box::new(ActionLeaf),
                        },
                        body,
                    }
                },
            )
            .collect();
        Node::Tabs(Tabs {
            id: self.ids.id(StableKey::Tabs(tabs_id)),
            metadata: ControlMetadata::new(""),
            selected,
            tabs: built,
        })
    }

    pub(crate) fn draft(
        &mut self,
        key: SettingsControlId,
        label: String,
        leaf: impl ControlLeaf,
        binding: ControlBinding,
    ) -> Draft {
        let stable = self.control_key(&key, &label);
        let id = self.ids.id(stable);
        Draft {
            view: Control {
                id,
                label: label.clone(),
                metadata: ControlMetadata::new(&label),
                display: ControlDisplay::Standard,
                leaf: Box::new(leaf),
            },
            binding,
            context: Vec::new(),
        }
    }
}

pub(crate) struct TabSpec {
    pub id: SettingsTabId,
    pub label: String,
    pub selected: bool,
    pub context: Vec<SettingsEvent>,
    pub children: Vec<Node>,
}

pub(crate) struct Draft {
    pub view: Control,
    pub binding: ControlBinding,
    pub context: Vec<SettingsEvent>,
}

impl Draft {
    pub fn finish(self, table: &mut BindingTable) -> Control {
        table.push(self.view.id, self.binding, self.context);
        self.view
    }
}

#[cfg(test)]
#[path = "screen_tests.rs"]
mod tests;
