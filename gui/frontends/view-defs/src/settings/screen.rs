//! Owned settings dialog state, projected through an L1 definition.
//!
//! The definition emits stable string keys per control; [`BindingTable`]
//! maps the minted [`ControlId`]s back to domain events. Native adapters
//! only see the projected screen. Row and tab identity comes from domain
//! content (field ids, slot ids, binding targets, messages) plus engine
//! occurrence suffixes; string keys are cheap to recompute for event binding.

use std::collections::HashMap;

use nerust_gui_viewmodel::settings::{
    SettingsViewModel,
    dto::{BindingRowView, BindingValueView, ChoiceView},
};
use nerust_settings_core::{
    editor::CaptureTarget,
    i18n::{UiText, notification_group_label, resolve_language, text},
    model::{language::AppLanguage, local::NotificationColor},
};
use wakui::Key;
use wakui::scene::control::{Control, ControlDisplay, ControlId, ControlInput, ControlScreen};
use wakui::scene::def::{
    Binding, BoolBinding, ChoicesDef, ContainerDef, ControlDef, LeafDef, NodeDef, NumBinding,
    PageDef, PolicyDef, RowTemplateDef, RowsDef, ScreenDef, SelectedDef, TabTemplateDef, TabsDef,
};
use wakui::scene::hints::LayoutHints;
use wakui::scene::inflate::{
    Choices, CustomLeaves, Flags, InflateCtx, Lists, RowPath, Texts, Value, cell_key, inflate,
    row_key,
};
use wakui::scene::leaf::{ActionLeaf, BindingLeaf, ChoiceLeaf, NumberLeaf, TextLeaf, ToggleLeaf};

use super::{InputPageSection, InputTab, SettingsController, SettingsEvent, SettingsPage};

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

/// Stable L1 key strings. Def assembly, the resolver-adjacent rebuild, and
/// tests share these so a typo fails in one place.
pub(crate) mod keys {
    pub const PAGE_GENERAL: &str = "page:general";
    pub const PAGE_INPUT: &str = "page:input";
    pub const PAGE_VIDEO: &str = "page:video";
    pub const PAGE_AUDIO: &str = "page:audio";
    pub const PAGE_NOTIFICATIONS: &str = "page:notifications";
    pub const PAGE_SYSTEM: &str = "page:system";
    pub const FOOTER: &str = "footer";
    pub const CANCEL: &str = "cancel";
    pub const SUBMIT: &str = "submit";

    pub const LANGUAGE: &str = "language";
    pub const STORAGE_POLICY: &str = "storage-policy";
    pub const STORAGE_DIR: &str = "storage-dir";
    pub const STORAGE_ERROR: &str = "storage-error";
    pub const FULLSCREEN: &str = "fullscreen";
    pub const SCALING: &str = "scaling";
    pub const VSYNC: &str = "vsync";
    pub const MUTE: &str = "mute";
    pub const VOLUME: &str = "volume";
    pub const SAMPLE_RATE: &str = "sample-rate";
    pub const LATENCY: &str = "latency";

    pub const GROUP_INFO: &str = "group:info";
    pub const GROUP_WARNING: &str = "group:warning";
    pub const GROUP_ERROR: &str = "group:error";
    pub const GROUP_DISPLAY: &str = "group:display";
    pub const GROUP_STATUS: &str = "group:status";
    pub const INFO_ENABLED: &str = "info-enabled";
    pub const INFO_DURATION: &str = "info-duration";
    pub const INFO_COLOR: &str = "info-color";
    pub const WARNING_ENABLED: &str = "warning-enabled";
    pub const WARNING_DURATION: &str = "warning-duration";
    pub const WARNING_COLOR: &str = "warning-color";
    pub const ERROR_ENABLED: &str = "error-enabled";
    pub const ERROR_PRESENTATION: &str = "error-presentation";
    pub const ERROR_COLOR: &str = "error-color";
    pub const POSITION: &str = "position";
    pub const AREA_MODE: &str = "area-mode";
    pub const STATUS_ENABLED: &str = "status-enabled";
    pub const STATUS_POSITION: &str = "status-position";
    pub const STATUS_AREA_MODE: &str = "status-area-mode";
    pub const STATUS_AREA_FOLLOWS: &str = "status-area-follows";

    pub const TABS_SYSTEMS: &str = "tabs:systems";
    pub const TABS_INPUTS: &str = "tabs:inputs";
    pub const TABS_SECTIONS: &str = "tabs";
    pub const LIST_FIELDS: &str = "fields";
    pub const LIST_CONFLICTS: &str = "conflicts";
    pub const LIST_SLOTS: &str = "slots";
    pub const LIST_ROWS: &str = "rows";
    pub const LIST_SHORTCUTS: &str = "shortcuts";
    pub const CELL: &str = "cell";
    pub const NO_SYSTEMS: &str = "no-systems";
}

/// Row content key, mirroring the old content-addressed identities.
pub(crate) fn binding_key(target: &CaptureTarget) -> String {
    match target {
        CaptureTarget::Binding {
            system,
            attachment,
            control,
        } => format!("binding:{system}:{attachment}:{control}"),
        CaptureTarget::Shortcut(action) => format!("shortcut:{action:?}"),
    }
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
        let language = self.language();
        let validation = self.validation_banner();
        let Self {
            ids,
            vm,
            page,
            system_tab_index,
            input_tab,
            input_section,
            storage_directory_input,
            error_message,
            ..
        } = self;
        let def = settings_def(
            language,
            *page,
            storage_directory_input,
            validation.storage.as_deref(),
            validation.can_submit,
            vm,
        );
        let resolver = SettingsResolver {
            language,
            vm,
            system_tab_index: *system_tab_index,
            input_tab: *input_tab,
            input_section: *input_section,
        };
        let mut ctx = ids.borrow_mut();
        let (mut screen, live) = inflate(&def, &mut ctx, |key| key.to_string(), &resolver);
        ctx.retain_rows(&live);
        let bindings = rebuild_table(vm, &mut ctx);
        screen.message = error_message
            .clone()
            .or_else(|| validation.headline.clone());
        let prompt = vm.capture.view.get().prompt.clone();
        screen.capture = wakui::scene::control::Capture {
            target: find_capturing(&screen),
            prompt,
        };
        SettingsScreen { screen, bindings }
    }
}

/// First capturing binding leaf wins; at most one captures at a time.
fn find_capturing(screen: &ControlScreen) -> Option<ControlId> {
    struct Search(Option<ControlId>);
    impl wakui::scene::control::ControlVisitor for Search {
        fn control(&mut self, control: &Control) {
            if self.0.is_none()
                && let Some(leaf) = control.downcast::<BindingLeaf>()
                && leaf.capturing
            {
                self.0 = Some(control.id);
            }
            wakui::scene::control::walk_control(control, self);
        }
    }
    let mut search = Search(None);
    screen.visit(&mut search);
    search.0
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
/// L1 control factories. Every string is a [`Binding`]; structural choices
/// (gates, positions, ranges) arrive as literals because the definition is
/// assembled per frame.
fn action(key: &str, label: Binding, display: ControlDisplay) -> ControlDef {
    ControlDef {
        key: key.into(),
        label,
        accessible: None,
        help: None,
        display,
        enabled: BoolBinding::Literal(true),
        visible: BoolBinding::Literal(true),
        hints: LayoutHints::default(),
        leaf: LeafDef::Action,
    }
}

fn note(key: &str, label: Binding, text: Binding, display: ControlDisplay) -> ControlDef {
    ControlDef {
        key: key.into(),
        label,
        accessible: None,
        help: None,
        display,
        enabled: BoolBinding::Literal(true),
        visible: BoolBinding::Literal(true),
        hints: LayoutHints::default(),
        leaf: LeafDef::Note { text },
    }
}

fn toggle(key: &str, label: Binding, value: bool) -> ControlDef {
    ControlDef {
        key: key.into(),
        label,
        accessible: None,
        help: None,
        display: ControlDisplay::Standard,
        enabled: BoolBinding::Literal(true),
        visible: BoolBinding::Literal(true),
        hints: LayoutHints::default(),
        leaf: LeafDef::Toggle {
            value: BoolBinding::Literal(value),
        },
    }
}

fn choice_labels<T: Clone + Eq>(choices: &[ChoiceView<T>]) -> Vec<String> {
    choices.iter().map(|c| c.label.clone()).collect()
}

fn choice_position<T: Clone + Eq>(choices: &[ChoiceView<T>], value: &T) -> Option<usize> {
    choices.iter().position(|c| &c.value == value)
}

fn choice_static(
    key: &str,
    label: Binding,
    labels: Vec<String>,
    selected: Option<usize>,
    enabled: bool,
    visible: bool,
) -> ControlDef {
    ControlDef {
        key: key.into(),
        label,
        accessible: None,
        help: None,
        display: ControlDisplay::Standard,
        enabled: BoolBinding::Literal(enabled),
        visible: BoolBinding::Literal(visible),
        hints: LayoutHints::default(),
        leaf: LeafDef::Choice {
            choices: ChoicesDef::Static(labels.into_iter().map(Binding::Literal).collect()),
            selected: match selected {
                Some(index) => SelectedDef::Index(index),
                None => SelectedDef::Index(usize::MAX),
            },
        },
    }
}

#[allow(clippy::too_many_arguments)]
fn number(
    key: &str,
    label: Binding,
    value: u32,
    min: u32,
    max: u32,
    step: u32,
    unit: &str,
    enabled: bool,
) -> ControlDef {
    ControlDef {
        key: key.into(),
        label,
        accessible: None,
        help: None,
        display: ControlDisplay::Standard,
        enabled: BoolBinding::Literal(enabled),
        visible: BoolBinding::Literal(true),
        hints: LayoutHints::default(),
        leaf: LeafDef::Number {
            value: NumBinding::Literal(value),
            min,
            max,
            step,
            unit: unit.into(),
        },
    }
}

fn text_input(
    key: &str,
    label: Binding,
    value: String,
    browse: Option<Binding>,
    enabled: bool,
    visible: bool,
) -> ControlDef {
    ControlDef {
        key: key.into(),
        label,
        accessible: None,
        help: None,
        display: ControlDisplay::Standard,
        enabled: BoolBinding::Literal(enabled),
        visible: BoolBinding::Literal(visible),
        hints: LayoutHints::default(),
        leaf: LeafDef::Text {
            value: Binding::Literal(value),
            placeholder: Binding::Literal(String::new()),
            browse_label: browse,
        },
    }
}

fn column(key: &str, children: Vec<NodeDef>) -> ContainerDef {
    ContainerDef {
        key: key.into(),
        label: Binding::Literal(String::new()),
        hints: LayoutHints {
            spacing: Some(12.0),
            indent: Some(0.0),
            ..LayoutHints::default()
        },
        header: None,
        policy: PolicyDef::Column,
        children,
    }
}

fn group(
    key: &str,
    title: Binding,
    header: Option<ControlDef>,
    header_accessible: Binding,
    children: Vec<NodeDef>,
) -> NodeDef {
    NodeDef::Container(Box::new(ContainerDef {
        key: key.into(),
        label: title,
        hints: LayoutHints {
            spacing: Some(8.0),
            indent: Some(16.0),
            ..LayoutHints::default()
        },
        header: header.map(|mut control| {
            control.accessible = Some(header_accessible);
            control
        }),
        policy: PolicyDef::Group,
        children,
    }))
}

fn settings_def(
    language: AppLanguage,
    page: SettingsPage,
    storage_input: &str,
    storage_error: Option<&str>,
    can_submit: bool,
    vm: &SettingsViewModel,
) -> ScreenDef {
    use keys::*;
    let t = |key: UiText| Binding::Localized(format!("{key:?}"));
    let page_title = |page: SettingsPage| match page {
        SettingsPage::General => t(UiText::General),
        SettingsPage::Input => t(UiText::Input),
        SettingsPage::Video => t(UiText::Video),
        SettingsPage::Audio => t(UiText::Audio),
        SettingsPage::Notifications => t(UiText::Notifications),
        SettingsPage::System => t(UiText::System),
    };
    let page_key = |page: SettingsPage| match page {
        SettingsPage::General => PAGE_GENERAL,
        SettingsPage::Input => PAGE_INPUT,
        SettingsPage::Video => PAGE_VIDEO,
        SettingsPage::Audio => PAGE_AUDIO,
        SettingsPage::Notifications => PAGE_NOTIFICATIONS,
        SettingsPage::System => PAGE_SYSTEM,
    };
    let pages = [
        SettingsPage::General,
        SettingsPage::Input,
        SettingsPage::Video,
        SettingsPage::Audio,
        SettingsPage::Notifications,
        SettingsPage::System,
    ];
    ScreenDef {
        title: t(UiText::Preferences),
        pages: pages
            .into_iter()
            .map(|id| {
                let children = match id {
                    SettingsPage::General => general_nodes(vm, storage_input, storage_error, &t),
                    SettingsPage::Input => input_page(),
                    SettingsPage::Video => video_nodes(vm, &t),
                    SettingsPage::Audio => audio_nodes(vm, &t),
                    SettingsPage::Notifications => notification_nodes(vm, &t, language),
                    SettingsPage::System => systems_page(vm),
                };
                PageDef {
                    key: page_key(id).into(),
                    label: page_title(id),
                    selected: page == id,
                    body: column(&format!("{}:body", page_key(id)), children),
                }
            })
            .collect(),
        footer: ContainerDef {
            key: FOOTER.into(),
            label: Binding::Literal(String::new()),
            hints: LayoutHints {
                spacing: Some(12.0),
                indent: Some(0.0),
                ..LayoutHints::default()
            },
            header: None,
            policy: PolicyDef::Row,
            children: vec![
                NodeDef::Control(Box::new(action(
                    CANCEL,
                    t(UiText::Cancel),
                    ControlDisplay::Secondary,
                ))),
                NodeDef::Control(Box::new({
                    let mut submit = action(SUBMIT, t(UiText::Ok), ControlDisplay::Primary);
                    submit.enabled = BoolBinding::Literal(can_submit);
                    submit
                })),
            ],
        },
    }
}

fn general_nodes(
    vm: &SettingsViewModel,
    storage_input: &str,
    storage_error: Option<&str>,
    t: &impl Fn(UiText) -> Binding,
) -> Vec<NodeDef> {
    use keys::*;
    let view = vm.general.view.get();
    let show = view.show_storage_directory;
    vec![
        NodeDef::Control(Box::new(choice_static(
            LANGUAGE,
            t(UiText::Language),
            choice_labels(&view.language_choices),
            choice_position(&view.language_choices, &view.language),
            true,
            true,
        ))),
        NodeDef::Control(Box::new(choice_static(
            STORAGE_POLICY,
            t(UiText::SaveStoragePolicy),
            choice_labels(&view.storage_policy_choices),
            choice_position(&view.storage_policy_choices, &view.storage_policy),
            true,
            true,
        ))),
        NodeDef::Control(Box::new(text_input(
            STORAGE_DIR,
            t(UiText::SaveStorageDirectory),
            storage_input.to_string(),
            Some(t(UiText::Browse)),
            show,
            show,
        ))),
        NodeDef::Control(Box::new({
            let mut error = note(
                STORAGE_ERROR,
                Binding::Literal(String::new()),
                Binding::Literal(storage_error.unwrap_or_default().to_string()),
                ControlDisplay::Standard,
            );
            error.visible = BoolBinding::Literal(show && storage_error.is_some());
            error
        })),
    ]
}

fn video_nodes(vm: &SettingsViewModel, t: &impl Fn(UiText) -> Binding) -> Vec<NodeDef> {
    use keys::*;
    let view = vm.video.view.get();
    vec![
        NodeDef::Control(Box::new(toggle(
            FULLSCREEN,
            t(UiText::FullscreenDefault),
            view.fullscreen_default,
        ))),
        NodeDef::Control(Box::new(choice_static(
            SCALING,
            t(UiText::Scaling),
            choice_labels(&view.scaling_choices),
            choice_position(&view.scaling_choices, &view.scaling),
            true,
            true,
        ))),
        NodeDef::Control(Box::new(toggle(VSYNC, t(UiText::Vsync), view.vsync))),
    ]
}

fn audio_nodes(vm: &SettingsViewModel, t: &impl Fn(UiText) -> Binding) -> Vec<NodeDef> {
    use keys::*;
    let view = vm.audio.view.get();
    vec![
        NodeDef::Control(Box::new(toggle(MUTE, t(UiText::Mute), view.muted))),
        NodeDef::Control(Box::new(number(
            VOLUME,
            t(UiText::MasterVolume),
            view.volume_percent.into(),
            0,
            100,
            1,
            "%",
            true,
        ))),
        NodeDef::Control(Box::new(choice_static(
            SAMPLE_RATE,
            t(UiText::SampleRate),
            choice_labels(&view.sample_rate_choices),
            choice_position(&view.sample_rate_choices, &view.sample_rate),
            true,
            true,
        ))),
        NodeDef::Control(Box::new(number(
            LATENCY,
            t(UiText::AudioLatency),
            view.latency_ms.into(),
            10,
            200,
            1,
            "ms",
            true,
        ))),
    ]
}

fn notification_nodes(
    vm: &SettingsViewModel,
    t: &impl Fn(UiText) -> Binding,
    language: AppLanguage,
) -> Vec<NodeDef> {
    use keys::*;
    let view = vm.notifications.view.get();
    let header_toggle = |key: &str, on: bool| ControlDef {
        key: key.into(),
        label: t(UiText::NotificationEnabled),
        accessible: None,
        help: None,
        display: ControlDisplay::Standard,
        enabled: BoolBinding::Literal(true),
        visible: BoolBinding::Literal(true),
        hints: LayoutHints::default(),
        leaf: LeafDef::Toggle {
            value: BoolBinding::Literal(on),
        },
    };
    let mut groups = Vec::new();
    let timed = |duration_key: &str,
                 color_key: &str,
                 on: bool,
                 duration: u32,
                 color: &NotificationColor,
                 colors: &[ChoiceView<NotificationColor>]| {
        vec![
            NodeDef::Control(Box::new(number(
                duration_key,
                t(UiText::NotificationDuration),
                duration,
                1000,
                30000,
                500,
                "ms",
                on,
            ))),
            NodeDef::Control(Box::new(choice_static(
                color_key,
                t(UiText::NotificationColor),
                choice_labels(colors),
                choice_position(colors, color),
                on,
                true,
            ))),
        ]
    };
    let presentations = [
        ChoiceView {
            value: true,
            label: text(language, UiText::NotificationErrorModal).to_string(),
        },
        ChoiceView {
            value: false,
            label: text(language, UiText::NotificationErrorOnScreen).to_string(),
        },
    ];
    groups.push(group(
        GROUP_INFO,
        t(UiText::NotificationInfo),
        Some(header_toggle(INFO_ENABLED, view.info_enabled)),
        t(UiText::NotificationInfo),
        timed(
            INFO_DURATION,
            INFO_COLOR,
            view.info_enabled,
            view.info_duration_ms,
            &view.info_color,
            &view.color_choices,
        ),
    ));
    groups.push(group(
        GROUP_WARNING,
        t(UiText::NotificationWarning),
        Some(header_toggle(WARNING_ENABLED, view.warning_enabled)),
        t(UiText::NotificationWarning),
        timed(
            WARNING_DURATION,
            WARNING_COLOR,
            view.warning_enabled,
            view.warning_duration_ms,
            &view.warning_color,
            &view.color_choices,
        ),
    ));
    groups.push(group(
        GROUP_ERROR,
        t(UiText::Error),
        Some(header_toggle(ERROR_ENABLED, view.error_enabled)),
        t(UiText::Error),
        vec![
            NodeDef::Control(Box::new(choice_static(
                ERROR_PRESENTATION,
                t(UiText::NotificationError),
                choice_labels(&presentations),
                choice_position(&presentations, &view.error_modal),
                view.error_enabled,
                true,
            ))),
            NodeDef::Control(Box::new(choice_static(
                ERROR_COLOR,
                t(UiText::NotificationColor),
                choice_labels(&view.color_choices),
                choice_position(&view.color_choices, &view.error_color),
                view.error_enabled && !view.error_modal,
                !view.error_modal,
            ))),
        ],
    ));
    groups.push(group(
        GROUP_DISPLAY,
        t(UiText::NotificationSectionDisplay),
        None,
        t(UiText::NotificationSectionDisplay),
        vec![
            NodeDef::Control(Box::new(choice_static(
                POSITION,
                t(UiText::NotificationPosition),
                choice_labels(&view.position_choices),
                choice_position(&view.position_choices, &view.position),
                true,
                true,
            ))),
            NodeDef::Control(Box::new(choice_static(
                AREA_MODE,
                t(UiText::NotificationAreaMode),
                choice_labels(&view.area_mode_choices),
                choice_position(&view.area_mode_choices, &view.area_mode),
                true,
                true,
            ))),
        ],
    ));
    let same_edge = view.position.same_edge(view.status_position);
    groups.push(group(
        GROUP_STATUS,
        t(UiText::NotificationSectionStatus),
        Some(header_toggle(STATUS_ENABLED, view.status_enabled)),
        t(UiText::NotificationStatus),
        vec![
            NodeDef::Control(Box::new(choice_static(
                STATUS_POSITION,
                t(UiText::NotificationStatusPosition),
                choice_labels(&view.position_choices),
                choice_position(&view.position_choices, &view.status_position),
                view.status_enabled,
                true,
            ))),
            NodeDef::Control(Box::new(choice_static(
                STATUS_AREA_MODE,
                t(UiText::NotificationStatusAreaMode),
                choice_labels(&view.area_mode_choices),
                choice_position(&view.area_mode_choices, &view.status_area_mode),
                view.status_enabled && !same_edge,
                !same_edge,
            ))),
            NodeDef::Control(Box::new({
                let mut follows = note(
                    STATUS_AREA_FOLLOWS,
                    t(UiText::NotificationStatusAreaMode),
                    t(UiText::NotificationStatusAreaFollows),
                    ControlDisplay::Standard,
                );
                follows.visible = BoolBinding::Literal(same_edge);
                follows
            })),
        ],
    ));
    groups
}
fn systems_page(vm: &SettingsViewModel) -> Vec<NodeDef> {
    use keys::*;
    let mut children = vec![];
    if vm.systems().is_empty() {
        children.push(NodeDef::Control(Box::new(note(
            NO_SYSTEMS,
            Binding::Literal(String::new()),
            Binding::Localized("no_systems".into()),
            ControlDisplay::Standard,
        ))));
    }
    children.push(NodeDef::Tabs(TabsDef::Bound {
        key: TABS_SYSTEMS.into(),
        source: "systems".into(),
        template: Box::new(TabTemplateDef {
            id_field: "system".into(),
            label: Binding::Field("label".into()),
            body: column(
                "pane",
                vec![NodeDef::Control(Box::new(ControlDef {
                    key: LIST_FIELDS.into(),
                    label: Binding::Literal(String::new()),
                    accessible: None,
                    help: None,
                    display: ControlDisplay::Standard,
                    enabled: BoolBinding::Literal(true),
                    visible: BoolBinding::Literal(true),
                    hints: LayoutHints::default(),
                    leaf: LeafDef::List {
                        columns: Vec::new(),
                        rows: RowsDef::Nested {
                            source: "fields".into(),
                            template: RowTemplateDef {
                                id_field: "field".into(),
                                cells: vec![ControlDef {
                                    key: "cell".into(),
                                    label: Binding::Field("label".into()),
                                    accessible: None,
                                    help: None,
                                    display: ControlDisplay::Standard,
                                    enabled: BoolBinding::Literal(true),
                                    visible: BoolBinding::Literal(true),
                                    hints: LayoutHints::default(),
                                    leaf: LeafDef::Choice {
                                        choices: ChoicesDef::Row {
                                            labels: "options".into(),
                                            selected: "selected".into(),
                                        },
                                        selected: SelectedDef::Index(0),
                                    },
                                }],
                            },
                        },
                    },
                }))],
            ),
        }),
        selected: SelectedDef::Bound("systems_selected".into()),
    }));
    children
}

fn binding_cell(key: &str) -> ControlDef {
    ControlDef {
        key: key.into(),
        label: Binding::Field("label".into()),
        accessible: None,
        help: None,
        display: ControlDisplay::Standard,
        enabled: BoolBinding::Literal(true),
        visible: BoolBinding::Literal(true),
        hints: LayoutHints::default(),
        leaf: LeafDef::Binding {
            text: Binding::Field("text".into()),
            capturing: BoolBinding::Field("capturing".into()),
            change_label: Binding::Localized(format!("{:?}", UiText::Change)),
            clear_label: Binding::Localized(format!("{:?}", UiText::Clear)),
        },
    }
}

fn sections_tabs() -> NodeDef {
    NodeDef::Tabs(TabsDef::Bound {
        key: keys::TABS_SECTIONS.into(),
        source: "sections".into(),
        template: Box::new(TabTemplateDef {
            id_field: "tab_id".into(),
            label: Binding::Field("tab_label".into()),
            body: column(
                "pane",
                vec![NodeDef::Control(Box::new(ControlDef {
                    key: keys::LIST_ROWS.into(),
                    label: Binding::Literal(String::new()),
                    accessible: None,
                    help: None,
                    display: ControlDisplay::Standard,
                    enabled: BoolBinding::Literal(true),
                    visible: BoolBinding::Literal(true),
                    hints: LayoutHints::default(),
                    leaf: LeafDef::List {
                        columns: Vec::new(),
                        rows: RowsDef::Nested {
                            source: "binding_rows".into(),
                            template: RowTemplateDef {
                                id_field: "id".into(),
                                cells: vec![binding_cell(keys::CELL)],
                            },
                        },
                    },
                }))],
            ),
        }),
        selected: SelectedDef::Bound("sections_selected".into()),
    })
}

fn input_page() -> Vec<NodeDef> {
    use keys::*;
    vec![NodeDef::Tabs(TabsDef::Bound {
        key: TABS_INPUTS.into(),
        source: "input_tabs".into(),
        template: Box::new(TabTemplateDef {
            id_field: "tab_id".into(),
            label: Binding::Field("tab_label".into()),
            body: column(
                "pane",
                vec![
                    NodeDef::Container(Box::new(column(
                        "conflicts",
                        vec![NodeDef::Control(Box::new(ControlDef {
                            key: LIST_CONFLICTS.into(),
                            label: Binding::Literal(String::new()),
                            accessible: None,
                            help: None,
                            display: ControlDisplay::Standard,
                            enabled: BoolBinding::Literal(true),
                            visible: BoolBinding::Literal(true),
                            hints: LayoutHints::default(),
                            leaf: LeafDef::List {
                                columns: Vec::new(),
                                rows: RowsDef::Nested {
                                    source: "conflicts".into(),
                                    template: RowTemplateDef {
                                        id_field: "message".into(),
                                        cells: vec![ControlDef {
                                            key: CELL.into(),
                                            label: Binding::Literal(String::new()),
                                            accessible: None,
                                            help: None,
                                            display: ControlDisplay::Standard,
                                            enabled: BoolBinding::Literal(true),
                                            visible: BoolBinding::Literal(true),
                                            hints: LayoutHints::default(),
                                            leaf: LeafDef::Note {
                                                text: Binding::Field("message".into()),
                                            },
                                        }],
                                    },
                                },
                            },
                        }))],
                    ))),
                    NodeDef::Container(Box::new(column(
                        "slots",
                        vec![NodeDef::Control(Box::new(ControlDef {
                            key: LIST_SLOTS.into(),
                            label: Binding::Literal(String::new()),
                            accessible: None,
                            help: None,
                            display: ControlDisplay::Standard,
                            enabled: BoolBinding::Literal(true),
                            visible: BoolBinding::Literal(true),
                            hints: LayoutHints::default(),
                            leaf: LeafDef::List {
                                columns: Vec::new(),
                                rows: RowsDef::Nested {
                                    source: "slots".into(),
                                    template: RowTemplateDef {
                                        id_field: "slot".into(),
                                        cells: vec![ControlDef {
                                            key: "cell".into(),
                                            label: Binding::Field("label".into()),
                                            accessible: None,
                                            help: Some(Binding::Field("help".into())),
                                            display: ControlDisplay::Standard,
                                            enabled: BoolBinding::Field("occupied_enabled".into()),
                                            visible: BoolBinding::Literal(true),
                                            hints: LayoutHints::default(),
                                            leaf: LeafDef::Choice {
                                                choices: ChoicesDef::Row {
                                                    labels: "options".into(),
                                                    selected: "selected".into(),
                                                },
                                                selected: SelectedDef::Index(0),
                                            },
                                        }],
                                    },
                                },
                            },
                        }))],
                    ))),
                    sections_tabs(),
                    NodeDef::Control(Box::new(ControlDef {
                        key: LIST_SHORTCUTS.into(),
                        label: Binding::Literal(String::new()),
                        accessible: None,
                        help: None,
                        display: ControlDisplay::Standard,
                        enabled: BoolBinding::Literal(true),
                        visible: BoolBinding::Literal(true),
                        hints: LayoutHints::default(),
                        leaf: LeafDef::List {
                            columns: Vec::new(),
                            rows: RowsDef::Nested {
                                source: "shortcut_rows".into(),
                                template: RowTemplateDef {
                                    id_field: "id".into(),
                                    cells: vec![binding_cell(CELL)],
                                },
                            },
                        },
                    })),
                ],
            ),
        }),
        selected: SelectedDef::Bound("inputs_selected".into()),
    })]
}
/// Outer-owned resolution for [`settings_def`]: captions plus the dynamic
/// tab/row sources. Screen-level strings stay [`Binding::Literal`] in the
/// definition; only localized captions and item-varying rows resolve here.
struct SettingsResolver<'m> {
    language: AppLanguage,
    vm: &'m SettingsViewModel,
    system_tab_index: Option<usize>,
    input_tab: Option<InputTab>,
    input_section: InputPageSection,
}

impl Texts for SettingsResolver<'_> {
    fn text(&self, key: &str) -> String {
        let t = |key: UiText| text(self.language, key).to_string();
        match key {
            "Preferences" => t(UiText::Preferences),
            "General" => t(UiText::General),
            "Input" => t(UiText::Input),
            "Video" => t(UiText::Video),
            "Audio" => t(UiText::Audio),
            "Notifications" => t(UiText::Notifications),
            "System" => t(UiText::System),
            "Language" => t(UiText::Language),
            "SaveStoragePolicy" => t(UiText::SaveStoragePolicy),
            "SaveStorageDirectory" => t(UiText::SaveStorageDirectory),
            "Browse" => t(UiText::Browse),
            "FullscreenDefault" => t(UiText::FullscreenDefault),
            "Scaling" => t(UiText::Scaling),
            "Vsync" => t(UiText::Vsync),
            "Mute" => t(UiText::Mute),
            "MasterVolume" => t(UiText::MasterVolume),
            "SampleRate" => t(UiText::SampleRate),
            "AudioLatency" => t(UiText::AudioLatency),
            "NotificationEnabled" => t(UiText::NotificationEnabled),
            "NotificationDuration" => t(UiText::NotificationDuration),
            "NotificationColor" => t(UiText::NotificationColor),
            "NotificationError" => t(UiText::NotificationError),
            "NotificationErrorModal" => t(UiText::NotificationErrorModal),
            "NotificationErrorOnScreen" => t(UiText::NotificationErrorOnScreen),
            "NotificationPosition" => t(UiText::NotificationPosition),
            "NotificationAreaMode" => t(UiText::NotificationAreaMode),
            "NotificationStatusPosition" => t(UiText::NotificationStatusPosition),
            "NotificationStatusAreaMode" => t(UiText::NotificationStatusAreaMode),
            "NotificationStatusAreaFollows" => t(UiText::NotificationStatusAreaFollows),
            "NotificationInfo" => t(UiText::NotificationInfo),
            "NotificationWarning" => t(UiText::NotificationWarning),
            "Error" => t(UiText::Error),
            "NotificationSectionDisplay" => t(UiText::NotificationSectionDisplay),
            "NotificationSectionStatus" => t(UiText::NotificationSectionStatus),
            "NotificationStatus" => t(UiText::NotificationStatus),
            "Cancel" => t(UiText::Cancel),
            "Ok" => t(UiText::Ok),
            "Change" => t(UiText::Change),
            "Clear" => t(UiText::Clear),
            "group_header_info" => {
                notification_group_label(self.language, &t(UiText::NotificationInfo))
            }
            "group_header_warning" => {
                notification_group_label(self.language, &t(UiText::NotificationWarning))
            }
            "group_header_error" => notification_group_label(self.language, &t(UiText::Error)),
            "group_header_display" => {
                notification_group_label(self.language, &t(UiText::NotificationSectionDisplay))
            }
            "group_header_status" => t(UiText::NotificationStatus),
            "no_systems" => match resolve_language(self.language) {
                AppLanguage::Japanese => "利用可能なシステムがありません".into(),
                _ => "No systems available".into(),
            },
            "slot_occupied" => match resolve_language(self.language) {
                AppLanguage::Japanese => "他のスロットで使用中".into(),
                _ => "Occupied by another slot".into(),
            },
            _ => String::new(),
        }
    }
}

impl Flags for SettingsResolver<'_> {}

impl Choices for SettingsResolver<'_> {
    fn selected(&self, source: &str) -> Option<usize> {
        match source {
            "systems_selected" => {
                let n = self.vm.systems().len();
                self.system_tab_index.filter(|i| *i < n)
            }
            "inputs_selected" => {
                let n = self.vm.inputs().len();
                match self.input_tab {
                    Some(InputTab::System(index)) if index < n => Some(index),
                    Some(InputTab::Shortcuts) => Some(n),
                    None if n == 0 => Some(0),
                    _ => None,
                }
            }
            "sections_selected" => {
                let InputPageSection::Attachment(selected) = self.input_section;
                Some(selected)
            }
            _ => None,
        }
    }
}

impl CustomLeaves for SettingsResolver<'_> {}

/// Stable tab identity shared by the resolver and the event rebuild so both
/// compute the same keys without duplicating the rule.
pub(crate) fn section_tab_id(
    system: &str,
    section_index: usize,
    rows: &[BindingRowView],
) -> String {
    match rows.iter().find_map(|row| match &row.target {
        CaptureTarget::Binding { attachment, .. } => Some(attachment.clone()),
        _ => None,
    }) {
        Some(attachment) => format!("attachment:{system}:{attachment}"),
        None => format!("empty:{system}:{section_index}"),
    }
}

impl Lists for SettingsResolver<'_> {
    fn list_len(&self, source: &str) -> usize {
        match source {
            "systems" => self.vm.systems().len(),
            "input_tabs" => self.vm.inputs().len() + 1,
            _ => 0,
        }
    }

    fn value(&self, source: &str, index: usize, path: &str) -> Value {
        match source {
            "systems" => match self.vm.systems().get(index) {
                Some(vm) => {
                    let view = vm.view.get();
                    match path {
                        "system" => Value::Text(view.system_id.to_string()),
                        "label" => Value::Text(view.label.clone()),
                        _ => Value::Text(String::new()),
                    }
                }
                None => Value::Text(String::new()),
            },
            "input_tabs" => {
                let n = self.vm.inputs().len();
                if index < n {
                    let view = self.vm.inputs()[index].view.get();
                    match path {
                        "tab_id" => Value::Text(view.system_id.to_string()),
                        "tab_label" => Value::Text(view.label.clone()),
                        _ => Value::Text(String::new()),
                    }
                } else if index == n {
                    let view = self.vm.shortcuts.view.get();
                    match path {
                        "tab_id" => Value::Text("shortcuts".into()),
                        "tab_label" => Value::Text(view.label.clone()),
                        _ => Value::Text(String::new()),
                    }
                } else {
                    Value::Text(String::new())
                }
            }
            _ => Value::Text(String::new()),
        }
    }

    fn nested_len(&self, source: &str, parent: &RowPath) -> usize {
        match source {
            "fields" => match parent.0.as_slice() {
                [(psource, pindex)] if psource == "systems" => {
                    self.vm.systems().get(*pindex).map_or(0, |vm| {
                        let view = vm.view.get();
                        view.fields.len()
                    })
                }
                _ => 0,
            },
            "conflicts" => match parent.0.as_slice() {
                [(psource, pindex)] if psource == "input_tabs" => {
                    self.vm.inputs().get(*pindex).map_or(0, |vm| {
                        let view = vm.view.get();
                        view.conflicts.len()
                    })
                }
                _ => 0,
            },
            "slots" => match parent.0.as_slice() {
                [(psource, pindex)] if psource == "input_tabs" => {
                    self.vm.inputs().get(*pindex).map_or(0, |vm| {
                        let view = vm.view.get();
                        view.slots.len()
                    })
                }
                _ => 0,
            },
            "sections" => match parent.0.as_slice() {
                [(psource, pindex)]
                    if psource == "input_tabs" && *pindex < self.vm.inputs().len() =>
                {
                    self.vm.inputs().get(*pindex).map_or(0, |vm| {
                        let view = vm.view.get();
                        view.sections.len()
                    })
                }
                _ => 0,
            },
            "binding_rows" => match parent.0.as_slice() {
                [(tsource, tindex), (ssource, sindex)]
                    if tsource == "input_tabs" && ssource == "sections" =>
                {
                    self.vm
                        .inputs()
                        .get(*tindex)
                        .and_then(|vm| vm.view.get().sections.get(*sindex).cloned())
                        .map_or(0, |section| section.rows.len())
                }
                _ => 0,
            },
            "shortcut_rows" => match parent.0.as_slice() {
                [(psource, pindex)]
                    if psource == "input_tabs" && *pindex == self.vm.inputs().len() =>
                {
                    self.vm.shortcuts.view.get().rows.len()
                }
                _ => 0,
            },
            _ => 0,
        }
    }

    fn nested_value(&self, source: &str, parent: &RowPath, index: usize, path: &str) -> Value {
        let no = Value::Text(String::new());
        match source {
            "fields" => match parent.0.as_slice() {
                [(psource, pindex)] if psource == "systems" => {
                    match self
                        .vm
                        .systems()
                        .get(*pindex)
                        .and_then(|vm| vm.view.get().fields.get(index).cloned())
                    {
                        Some(field) => match path {
                            "field" => Value::Text(field.id.as_str().to_string()),
                            "label" => Value::Text(field.label.clone()),
                            "options" => {
                                Value::List(field.choices.iter().map(|c| c.label.clone()).collect())
                            }
                            "selected" => Value::Num(
                                field
                                    .choices
                                    .iter()
                                    .position(|c| c.value == field.selected)
                                    .unwrap_or(usize::MAX) as u32,
                            ),
                            _ => no,
                        },
                        None => no,
                    }
                }
                _ => no,
            },
            "conflicts" => match parent.0.as_slice() {
                [(psource, pindex)] if psource == "input_tabs" => {
                    match self
                        .vm
                        .inputs()
                        .get(*pindex)
                        .and_then(|vm| vm.view.get().conflicts.get(index).cloned())
                    {
                        Some(conflict) => match path {
                            "message" => Value::Text(conflict.message.clone()),
                            _ => no,
                        },
                        None => no,
                    }
                }
                _ => no,
            },
            "slots" => match parent.0.as_slice() {
                [(psource, pindex)] if psource == "input_tabs" => {
                    match self
                        .vm
                        .inputs()
                        .get(*pindex)
                        .and_then(|vm| vm.view.get().slots.get(index).cloned())
                    {
                        Some(slot) => match path {
                            "slot" => Value::Text(slot.slot_id.as_str().to_string()),
                            "label" => Value::Text(slot.label.clone()),
                            "options" => {
                                Value::List(slot.choices.iter().map(|c| c.label.clone()).collect())
                            }
                            "selected" => Value::Num(
                                slot.choices
                                    .iter()
                                    .position(|c| c.value == slot.selected_profile_id)
                                    .unwrap_or(usize::MAX) as u32,
                            ),
                            "occupied_enabled" => Value::Bool(!slot.occupied_by_other_slot),
                            "help" => {
                                if slot.occupied_by_other_slot {
                                    Value::Text(match resolve_language(self.language) {
                                        AppLanguage::Japanese => "他のスロットで使用中".into(),
                                        _ => "Occupied by another slot".into(),
                                    })
                                } else {
                                    Value::Text(String::new())
                                }
                            }
                            _ => no,
                        },
                        None => no,
                    }
                }
                _ => no,
            },
            "sections" => match parent.0.as_slice() {
                [(psource, pindex)] if psource == "input_tabs" => {
                    if *pindex >= self.vm.inputs().len() {
                        return no;
                    }
                    match self
                        .vm
                        .inputs()
                        .get(*pindex)
                        .and_then(|vm| vm.view.get().sections.get(index).cloned())
                    {
                        Some(section) => {
                            let system = self.vm.inputs()[*pindex].view.get().system_id.to_string();
                            match path {
                                "tab_id" => {
                                    Value::Text(section_tab_id(&system, index, &section.rows))
                                }
                                "tab_label" => Value::Text(section.label.clone()),
                                _ => no,
                            }
                        }
                        None => no,
                    }
                }
                _ => no,
            },
            "binding_rows" => match parent.0.as_slice() {
                [(tsource, tindex), (ssource, sindex)]
                    if tsource == "input_tabs" && ssource == "sections" =>
                {
                    match self.vm.inputs().get(*tindex).and_then(|vm| {
                        vm.view
                            .get()
                            .sections
                            .get(*sindex)
                            .and_then(|section| section.rows.get(index).cloned())
                    }) {
                        Some(row) => binding_value(&row, path),
                        None => no,
                    }
                }
                _ => no,
            },
            "shortcut_rows" => match parent.0.as_slice() {
                [(psource, pindex)] if psource == "input_tabs" => {
                    if *pindex != self.vm.inputs().len() {
                        return no;
                    }
                    match self.vm.shortcuts.view.get().rows.get(index) {
                        Some(row) => binding_value(row, path),
                        None => no,
                    }
                }
                _ => no,
            },
            _ => no,
        }
    }
}

fn binding_value(row: &BindingRowView, path: &str) -> Value {
    let no = Value::Text(String::new());
    match path {
        "id" => Value::Text(binding_key(&row.target)),
        "label" => Value::Text(row.label.clone()),
        "text" => Value::Text(match &row.value {
            BindingValueView::Unbound(text)
            | BindingValueView::Bound(text)
            | BindingValueView::Capturing(text) => text.clone(),
        }),
        "capturing" => Value::Bool(matches!(row.value, BindingValueView::Capturing(_))),
        _ => no,
    }
}
/// Occurrence counter mirroring the engine: fresh map per parent group.
fn occ(seen: &mut HashMap<String, usize>, identity: &str) -> usize {
    let count = seen.get(identity).copied().unwrap_or(0);
    seen.insert(identity.to_string(), count + 1);
    count
}

fn namespaced_join(ns: &str, key: &str) -> String {
    if ns.is_empty() {
        key.to_string()
    } else {
        cell_key(ns, key)
    }
}

struct Rebuild<'a> {
    ids: &'a mut InflateCtx<String>,
    table: BindingTable,
}

impl Rebuild<'_> {
    fn push(&mut self, key: &str, binding: ControlBinding, context: Vec<SettingsEvent>) {
        let id = self.ids.ids.id(key.to_string());
        self.table.push(id, binding, context);
    }
}

/// Rebuild the event table from stable L1 keys. Static controls resolve
/// directly; dynamic rows recompute the engine's namespaced keys with the
/// same per-parent occurrence discipline.
fn rebuild_table(vm: &SettingsViewModel, ctx: &mut InflateCtx<String>) -> BindingTable {
    use keys::*;
    let mut re = Rebuild {
        ids: ctx,
        table: BindingTable::default(),
    };
    // Static chrome: Choice events fan out per option like the old drafts.
    {
        let view = vm.general.view.get();
        re.push(
            LANGUAGE,
            ControlBinding::Choice(
                view.language_choices
                    .iter()
                    .map(|c| SettingsEvent::SetLanguage(c.clone()))
                    .collect(),
            ),
            Vec::new(),
        );
        re.push(
            STORAGE_POLICY,
            ControlBinding::Choice(
                view.storage_policy_choices
                    .iter()
                    .map(|c| SettingsEvent::SetStoragePolicy(c.clone()))
                    .collect(),
            ),
            Vec::new(),
        );
        re.push(
            STORAGE_DIR,
            ControlBinding::Text(SettingsEvent::SetStorageDirectory),
            Vec::new(),
        );
        re.push(STORAGE_ERROR, ControlBinding::None, Vec::new());
    }
    {
        let view = vm.video.view.get();
        let _ = view;
        re.push(
            FULLSCREEN,
            ControlBinding::Toggle(SettingsEvent::ToggleFullscreenDefault),
            Vec::new(),
        );
        re.push(
            SCALING,
            ControlBinding::Choice(
                view.scaling_choices
                    .iter()
                    .map(|c| SettingsEvent::SetScaling(c.clone()))
                    .collect(),
            ),
            Vec::new(),
        );
        re.push(
            VSYNC,
            ControlBinding::Toggle(SettingsEvent::ToggleVsync),
            Vec::new(),
        );
    }
    {
        let view = vm.audio.view.get();
        re.push(
            MUTE,
            ControlBinding::Toggle(SettingsEvent::ToggleMute),
            Vec::new(),
        );
        re.push(
            VOLUME,
            ControlBinding::Number(|v| SettingsEvent::SetVolume(v as u8)),
            Vec::new(),
        );
        re.push(
            SAMPLE_RATE,
            ControlBinding::Choice(
                view.sample_rate_choices
                    .iter()
                    .map(|c| SettingsEvent::SetSampleRate(c.clone()))
                    .collect(),
            ),
            Vec::new(),
        );
        re.push(
            LATENCY,
            ControlBinding::Number(|v| SettingsEvent::SetLatency(v as u16)),
            Vec::new(),
        );
    }
    {
        let view = vm.notifications.view.get();
        re.push(
            INFO_DURATION,
            ControlBinding::Number(SettingsEvent::SetInfoDuration),
            Vec::new(),
        );
        re.push(
            INFO_COLOR,
            ControlBinding::Choice(
                view.color_choices
                    .iter()
                    .map(|c| SettingsEvent::SetInfoColor(c.clone()))
                    .collect(),
            ),
            Vec::new(),
        );
        re.push(
            WARNING_DURATION,
            ControlBinding::Number(SettingsEvent::SetWarningDuration),
            Vec::new(),
        );
        re.push(
            WARNING_COLOR,
            ControlBinding::Choice(
                view.color_choices
                    .iter()
                    .map(|c| SettingsEvent::SetWarningColor(c.clone()))
                    .collect(),
            ),
            Vec::new(),
        );
        re.push(
            ERROR_PRESENTATION,
            ControlBinding::Choice(vec![
                SettingsEvent::ToggleErrorModal(true),
                SettingsEvent::ToggleErrorModal(false),
            ]),
            Vec::new(),
        );
        re.push(
            ERROR_COLOR,
            ControlBinding::Choice(
                view.color_choices
                    .iter()
                    .map(|c| SettingsEvent::SetErrorColor(c.clone()))
                    .collect(),
            ),
            Vec::new(),
        );
        re.push(
            POSITION,
            ControlBinding::Choice(
                view.position_choices
                    .iter()
                    .map(|c| SettingsEvent::SetPosition(c.clone()))
                    .collect(),
            ),
            Vec::new(),
        );
        re.push(
            AREA_MODE,
            ControlBinding::Choice(
                view.area_mode_choices
                    .iter()
                    .map(|c| SettingsEvent::SetAreaMode(c.clone()))
                    .collect(),
            ),
            Vec::new(),
        );
        re.push(
            STATUS_POSITION,
            ControlBinding::Choice(
                view.position_choices
                    .iter()
                    .map(|c| SettingsEvent::SetStatusPosition(c.clone()))
                    .collect(),
            ),
            Vec::new(),
        );
        re.push(
            STATUS_AREA_MODE,
            ControlBinding::Choice(
                view.area_mode_choices
                    .iter()
                    .map(|c| SettingsEvent::SetStatusAreaMode(c.clone()))
                    .collect(),
            ),
            Vec::new(),
        );
        re.push(STATUS_AREA_FOLLOWS, ControlBinding::None, Vec::new());
        for (key, toggle) in [
            (
                INFO_ENABLED,
                SettingsEvent::ToggleInfoEnabled as fn(bool) -> SettingsEvent,
            ),
            (
                WARNING_ENABLED,
                SettingsEvent::ToggleWarningEnabled as fn(bool) -> SettingsEvent,
            ),
            (
                ERROR_ENABLED,
                SettingsEvent::ToggleErrorEnabled as fn(bool) -> SettingsEvent,
            ),
            (
                STATUS_ENABLED,
                SettingsEvent::ToggleStatusEnabled as fn(bool) -> SettingsEvent,
            ),
        ] {
            re.push(key, ControlBinding::Toggle(toggle), Vec::new());
        }
    }
    // Pages and footer.
    for p in [
        SettingsPage::General,
        SettingsPage::Input,
        SettingsPage::Video,
        SettingsPage::Audio,
        SettingsPage::Notifications,
        SettingsPage::System,
    ] {
        let key = match p {
            SettingsPage::General => PAGE_GENERAL,
            SettingsPage::Input => PAGE_INPUT,
            SettingsPage::Video => PAGE_VIDEO,
            SettingsPage::Audio => PAGE_AUDIO,
            SettingsPage::Notifications => PAGE_NOTIFICATIONS,
            SettingsPage::System => PAGE_SYSTEM,
        };
        re.push(
            &format!("{key}/select"),
            ControlBinding::Action(SettingsEvent::SelectPage(p)),
            Vec::new(),
        );
    }
    if vm.systems().is_empty() {
        re.push(NO_SYSTEMS, ControlBinding::None, Vec::new());
    }
    re.push(
        CANCEL,
        ControlBinding::Action(SettingsEvent::Cancel),
        Vec::new(),
    );
    re.push(
        SUBMIT,
        ControlBinding::Action(SettingsEvent::Submit),
        Vec::new(),
    );
    // Systems tabs with nested field rows.
    {
        let mut seen = HashMap::new();
        for (index, system) in vm.systems().iter().enumerate() {
            let view = system.view.get();
            let sysid = view.system_id.to_string();
            let tabns = row_key("systems", &sysid, occ(&mut seen, &sysid));
            re.push(
                &cell_key(&tabns, "select"),
                ControlBinding::Action(SettingsEvent::SelectSystemTab(index)),
                Vec::new(),
            );
            re.push(
                &cell_key(&tabns, LIST_FIELDS),
                ControlBinding::None,
                Vec::new(),
            );
            let mut fseen = HashMap::new();
            for field in &view.fields {
                let fid = field.id.as_str().to_string();
                let rowns =
                    namespaced_join(&tabns, &row_key("fields", &fid, occ(&mut fseen, &fid)));
                re.push(
                    &cell_key(&rowns, CELL),
                    ControlBinding::Choice(
                        field
                            .choices
                            .iter()
                            .map(|c| SettingsEvent::SetSystemChoice(fid.clone(), c.clone()))
                            .collect(),
                    ),
                    vec![SettingsEvent::SelectSystemTab(index)],
                );
            }
        }
    }
    // Input tabs (systems union shortcuts) with nested sections and rows.
    {
        let n = vm.inputs().len();
        let mut seen = HashMap::new();
        for tab in 0..=n {
            let (tabid, context_select) = if tab < n {
                let sysid = vm.inputs()[tab].view.get().system_id.to_string();
                (sysid, SettingsEvent::SelectInputTab(tab))
            } else {
                ("shortcuts".to_string(), SettingsEvent::SelectInputTab(n))
            };
            let tabns = row_key("input_tabs", &tabid, occ(&mut seen, &tabid));
            re.push(
                &cell_key(&tabns, "select"),
                ControlBinding::Action(context_select.clone()),
                Vec::new(),
            );
            if tab < n {
                let view = vm.inputs()[tab].view.get();
                re.push(
                    &cell_key(&tabns, LIST_CONFLICTS),
                    ControlBinding::None,
                    Vec::new(),
                );
                re.push(
                    &cell_key(&tabns, LIST_SLOTS),
                    ControlBinding::None,
                    Vec::new(),
                );
                re.push(
                    &cell_key(&tabns, LIST_SHORTCUTS),
                    ControlBinding::None,
                    Vec::new(),
                );
                let mut cseen = HashMap::new();
                for conflict in &view.conflicts {
                    let rowns = namespaced_join(
                        &tabns,
                        &row_key(
                            "conflicts",
                            &conflict.message,
                            occ(&mut cseen, &conflict.message),
                        ),
                    );
                    re.push(&cell_key(&rowns, CELL), ControlBinding::None, Vec::new());
                }
                let mut sseen = HashMap::new();
                for slot in &view.slots {
                    let slotid = slot.slot_id.as_str().to_string();
                    let rowns = namespaced_join(
                        &tabns,
                        &row_key("slots", &slotid, occ(&mut sseen, &slotid)),
                    );
                    re.push(
                        &cell_key(&rowns, CELL),
                        ControlBinding::Choice(
                            slot.choices
                                .iter()
                                .map(|c| SettingsEvent::SetControllerSlot {
                                    slot: slot.slot_id,
                                    controller_id: c.value.clone(),
                                })
                                .collect(),
                        ),
                        vec![SettingsEvent::SelectInputTab(tab)],
                    );
                }
                let mut secseen = HashMap::new();
                for (section_index, section) in view.sections.iter().enumerate() {
                    let system = view.system_id.to_string();
                    let tabid = section_tab_id(&system, section_index, &section.rows);
                    let secns = namespaced_join(
                        &tabns,
                        &row_key("sections", &tabid, occ(&mut secseen, &tabid)),
                    );
                    re.push(
                        &cell_key(&secns, LIST_ROWS),
                        ControlBinding::None,
                        Vec::new(),
                    );
                    re.push(
                        &cell_key(&secns, "select"),
                        ControlBinding::Action(SettingsEvent::SelectInputSection(
                            InputPageSection::Attachment(section_index),
                        )),
                        vec![SettingsEvent::SelectInputTab(tab)],
                    );
                    let mut rseen = HashMap::new();
                    for row in &section.rows {
                        let id = binding_key(&row.target);
                        let rowns = namespaced_join(
                            &secns,
                            &row_key("binding_rows", &id, occ(&mut rseen, &id)),
                        );
                        re.push(
                            &cell_key(&rowns, CELL),
                            ControlBinding::Capture(row.target.clone()),
                            Vec::new(),
                        );
                    }
                }
            } else {
                re.push(
                    &cell_key(&tabns, LIST_CONFLICTS),
                    ControlBinding::None,
                    Vec::new(),
                );
                re.push(
                    &cell_key(&tabns, LIST_SLOTS),
                    ControlBinding::None,
                    Vec::new(),
                );
                re.push(
                    &cell_key(&tabns, LIST_SHORTCUTS),
                    ControlBinding::None,
                    Vec::new(),
                );
                let rows = vm.shortcuts.view.get().rows;
                let mut rseen = HashMap::new();
                for row in &rows {
                    let id = binding_key(&row.target);
                    let rowns = namespaced_join(
                        &tabns,
                        &row_key("shortcut_rows", &id, occ(&mut rseen, &id)),
                    );
                    re.push(
                        &cell_key(&rowns, CELL),
                        ControlBinding::Capture(row.target.clone()),
                        Vec::new(),
                    );
                }
            }
        }
    }
    re.table
}

#[cfg(test)]
#[path = "screen_tests.rs"]
mod tests;
