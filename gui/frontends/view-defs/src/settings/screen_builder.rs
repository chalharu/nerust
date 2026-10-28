use nerust_gui_viewmodel::settings::dto::{BindingRowView, ChoiceView};
use nerust_settings_core::i18n::{UiText, notification_group_label, resolve_language, text};

use super::*;
use crate::settings::{InputPageSection, InputTab, SettingsEvent as E};

type Id = SettingsControlId;
type Kind = SettingsControlKind;

fn metadata(label: &str) -> SettingsMetadata {
    SettingsMetadata {
        enabled: true,
        visible: true,
        accessible_label: label.into(),
        help: None,
    }
}

fn control(id: Id, label: String, kind: Kind, binding: ControlBinding) -> Draft {
    Draft {
        view: SettingsControl {
            id,
            metadata: metadata(&label),
            label,
            display: SettingsDisplay::Standard,
            kind,
        },
        binding,
        context: Vec::new(),
    }
}

/// View/binding pair under construction. `finish` registers the binding
/// into the screen table and returns the pure view for the tree.
struct Draft {
    view: SettingsControl,
    binding: ControlBinding,
    context: Vec<E>,
}

impl Draft {
    fn finish(self, table: &mut BindingTable) -> SettingsControl {
        table.push(self.view.id.clone(), self.binding, self.context);
        self.view
    }
}

fn layout(id: SettingsLayoutId, children: Vec<SettingsNode>) -> SettingsLayout {
    SettingsLayout {
        id,
        label: String::new(),
        metadata: metadata(""),
        spacing: 12,
        indent: 0,
        header: None,
        children,
    }
}

fn row(control: SettingsControl) -> SettingsNode {
    SettingsNode::Row(layout(
        SettingsLayoutId::Row(control.id.clone()),
        vec![SettingsNode::Control(control)],
    ))
}

fn toggle(id: Id, label: String, value: bool, event: fn(bool) -> E) -> Draft {
    control(
        id,
        label,
        Kind::Toggle { value },
        ControlBinding::Toggle(event),
    )
}

fn choice<T: Clone + Eq>(
    id: Id,
    label: String,
    choices: &[ChoiceView<T>],
    value: &T,
    event: impl Fn(ChoiceView<T>) -> E,
) -> Draft {
    control(
        id,
        label,
        Kind::Choice {
            choices: choices.iter().map(|c| c.label.clone()).collect(),
            selected: choices.iter().position(|c| &c.value == value),
        },
        ControlBinding::Choice(choices.iter().cloned().map(event).collect()),
    )
}

fn number(id: Id, label: String, value: u32, event: fn(u32) -> E) -> Draft {
    let (min, max, step, unit) = match id {
        Id::Volume => (0, 100, 1, "%"),
        Id::Latency => (10, 200, 1, "ms"),
        _ => (1000, 30000, 500, "ms"),
    };
    control(
        id,
        label,
        Kind::Number {
            value,
            min,
            max,
            step,
            unit: unit.into(),
            formatted: if unit == "%" {
                format!("{value}%")
            } else {
                format!("{value} {unit}")
            },
        },
        ControlBinding::Number(event),
    )
}

fn note(id: Id, label: String, value: String) -> Draft {
    let accessible_label = if label.is_empty() {
        value.clone()
    } else {
        format!("{label}: {value}")
    };
    let mut result = control(id, label, Kind::Note { text: value }, ControlBinding::None);
    result.view.metadata.accessible_label = accessible_label;
    result.view.display = SettingsDisplay::Note;
    result
}

fn action(id: Id, label: String, event: E) -> Draft {
    control(id, label, Kind::Action, ControlBinding::Action(event))
}

fn enabled(mut draft: Draft, enabled: bool) -> Draft {
    draft.view.metadata.enabled = enabled;
    draft
}

fn visible(mut draft: Draft, visible: bool) -> Draft {
    draft.view.metadata.visible = visible;
    draft
}

pub(super) fn build(controller: &SettingsController) -> SettingsScreen {
    let language = controller.language();
    let t = |key| text(language, key).to_string();
    let validation = controller.validation_banner();
    let mut table = BindingTable::default();
    let definitions = [
        (
            SettingsPage::General,
            UiText::General,
            general(controller, &mut table),
        ),
        (
            SettingsPage::Input,
            UiText::Input,
            input(controller, &mut table),
        ),
        (
            SettingsPage::Video,
            UiText::Video,
            video(controller, &mut table),
        ),
        (
            SettingsPage::Audio,
            UiText::Audio,
            audio(controller, &mut table),
        ),
        (
            SettingsPage::Notifications,
            UiText::Notifications,
            notifications(controller, &mut table),
        ),
        (
            SettingsPage::System,
            UiText::System,
            systems(controller, &mut table),
        ),
    ];
    let mut cancel = action(Id::Cancel, t(UiText::Cancel), E::Cancel);
    cancel.view.display = SettingsDisplay::Secondary;
    let mut submit = enabled(
        action(Id::Submit, t(UiText::Ok), E::Submit),
        validation.can_submit,
    );
    submit.view.display = SettingsDisplay::Primary;
    SettingsScreen {
        title: t(UiText::Preferences),
        language,
        pages: definitions
            .into_iter()
            .map(|(id, key, children)| SettingsScreenPage {
                id,
                label: t(key),
                selected: controller.page == id,
                select: action(Id::Page(id), t(key), E::SelectPage(id)).finish(&mut table),
                body: layout(SettingsLayoutId::Page(id), children),
            })
            .collect(),
        message: controller
            .error_message
            .clone()
            .or_else(|| validation.headline.clone()),
        validation,
        error_message: controller.error_message.clone(),
        capture: controller.vm.capture.view.get(),
        footer: layout(
            SettingsLayoutId::Footer,
            vec![
                SettingsNode::Control(cancel.finish(&mut table)),
                SettingsNode::Control(submit.finish(&mut table)),
            ],
        ),
        bindings: table,
    }
}

fn general(controller: &SettingsController, table: &mut BindingTable) -> Vec<SettingsNode> {
    let view = controller.vm.general.view.get();
    let t = |key| text(view.language, key).to_string();
    let storage = control(
        Id::StorageDirectory,
        t(UiText::SaveStorageDirectory),
        Kind::Text {
            // Native edit buffer is controller state, not another settings draft.
            value: controller.storage_directory_input.clone(),
            placeholder: String::new(),
            browse_label: Some(t(UiText::Browse)),
        },
        ControlBinding::Text(E::SetStorageDirectory),
    );
    let error = controller.validation_banner().storage;
    vec![
        row(choice(
            Id::Language,
            t(UiText::Language),
            &view.language_choices,
            &view.language,
            E::SetLanguage,
        )
        .finish(table)),
        row(choice(
            Id::StoragePolicy,
            t(UiText::SaveStoragePolicy),
            &view.storage_policy_choices,
            &view.storage_policy,
            E::SetStoragePolicy,
        )
        .finish(table)),
        row(visible(
            enabled(storage, view.show_storage_directory),
            view.show_storage_directory,
        )
        .finish(table)),
        row(visible(
            note(
                Id::StorageError,
                String::new(),
                error.clone().unwrap_or_default(),
            ),
            view.show_storage_directory && error.is_some(),
        )
        .finish(table)),
    ]
}

fn video(controller: &SettingsController, table: &mut BindingTable) -> Vec<SettingsNode> {
    let view = controller.vm.video.view.get();
    let t = |key| text(controller.language(), key).to_string();
    vec![
        row(toggle(
            Id::Fullscreen,
            t(UiText::FullscreenDefault),
            view.fullscreen_default,
            E::ToggleFullscreenDefault,
        )
        .finish(table)),
        row(choice(
            Id::Scaling,
            t(UiText::Scaling),
            &view.scaling_choices,
            &view.scaling,
            E::SetScaling,
        )
        .finish(table)),
        row(toggle(Id::Vsync, t(UiText::Vsync), view.vsync, E::ToggleVsync).finish(table)),
    ]
}

fn audio(controller: &SettingsController, table: &mut BindingTable) -> Vec<SettingsNode> {
    let view = controller.vm.audio.view.get();
    let t = |key| text(controller.language(), key).to_string();
    vec![
        row(toggle(Id::Mute, t(UiText::Mute), view.muted, E::ToggleMute).finish(table)),
        row(number(
            Id::Volume,
            t(UiText::MasterVolume),
            view.volume_percent.into(),
            |v| E::SetVolume(v as u8),
        )
        .finish(table)),
        row(choice(
            Id::SampleRate,
            t(UiText::SampleRate),
            &view.sample_rate_choices,
            &view.sample_rate,
            E::SetSampleRate,
        )
        .finish(table)),
        row(number(
            Id::Latency,
            t(UiText::AudioLatency),
            view.latency_ms.into(),
            |v| E::SetLatency(v as u16),
        )
        .finish(table)),
    ]
}

fn group(
    language: AppLanguage,
    id: NotificationGroup,
    title: UiText,
    header: Option<SettingsControl>,
    children: Vec<SettingsNode>,
) -> SettingsNode {
    let mut group = layout(SettingsLayoutId::Notification(id), children);
    group.label = text(language, title).into();
    group.metadata.accessible_label = group.label.clone();
    group.spacing = 8;
    group.indent = 16;
    group.header = header.map(|mut header| {
        header.metadata.accessible_label = if id == NotificationGroup::Status {
            text(language, UiText::NotificationStatus).into()
        } else {
            notification_group_label(language, &group.label)
        };
        header
    });
    SettingsNode::Group(group)
}

fn notifications(controller: &SettingsController, table: &mut BindingTable) -> Vec<SettingsNode> {
    let view = controller.vm.notifications.view.get();
    let lang = controller.language();
    let t = |key| text(lang, key).to_string();
    let mut groups = Vec::new();
    // Both timed levels intentionally share the same structure and range policy.
    for (
        group_id,
        title,
        enabled_id,
        duration_id,
        color_id,
        on,
        duration,
        color,
        toggle_event,
        duration_event,
        color_event,
    ) in [
        (
            NotificationGroup::Info,
            UiText::NotificationInfo,
            Id::InfoEnabled,
            Id::InfoDuration,
            Id::InfoColor,
            view.info_enabled,
            view.info_duration_ms,
            view.info_color,
            E::ToggleInfoEnabled as fn(bool) -> E,
            E::SetInfoDuration as fn(u32) -> E,
            E::SetInfoColor as fn(ChoiceView<_>) -> E,
        ),
        (
            NotificationGroup::Warning,
            UiText::NotificationWarning,
            Id::WarningEnabled,
            Id::WarningDuration,
            Id::WarningColor,
            view.warning_enabled,
            view.warning_duration_ms,
            view.warning_color,
            E::ToggleWarningEnabled,
            E::SetWarningDuration,
            E::SetWarningColor,
        ),
    ] {
        groups.push(group(
            lang,
            group_id,
            title,
            Some(
                toggle(enabled_id, t(UiText::NotificationEnabled), on, toggle_event).finish(table),
            ),
            vec![
                row(enabled(
                    number(
                        duration_id,
                        t(UiText::NotificationDuration),
                        duration,
                        duration_event,
                    ),
                    on,
                )
                .finish(table)),
                row(enabled(
                    choice(
                        color_id,
                        t(UiText::NotificationColor),
                        &view.color_choices,
                        &color,
                        color_event,
                    ),
                    on,
                )
                .finish(table)),
            ],
        ));
    }
    let presentations = vec![
        ChoiceView {
            value: true,
            label: t(UiText::NotificationErrorModal),
        },
        ChoiceView {
            value: false,
            label: t(UiText::NotificationErrorOnScreen),
        },
    ];
    groups.push(group(
        lang,
        NotificationGroup::Error,
        UiText::Error,
        Some(
            toggle(
                Id::ErrorEnabled,
                t(UiText::NotificationEnabled),
                view.error_enabled,
                E::ToggleErrorEnabled,
            )
            .finish(table),
        ),
        vec![
            row(enabled(
                choice(
                    Id::ErrorPresentation,
                    t(UiText::NotificationError),
                    &presentations,
                    &view.error_modal,
                    |c| E::ToggleErrorModal(c.value),
                ),
                view.error_enabled,
            )
            .finish(table)),
            row(visible(
                enabled(
                    choice(
                        Id::ErrorColor,
                        t(UiText::NotificationColor),
                        &view.color_choices,
                        &view.error_color,
                        E::SetErrorColor,
                    ),
                    view.error_enabled && !view.error_modal,
                ),
                !view.error_modal,
            )
            .finish(table)),
        ],
    ));
    groups.push(group(
        lang,
        NotificationGroup::Display,
        UiText::NotificationSectionDisplay,
        None,
        vec![
            row(choice(
                Id::Position,
                t(UiText::NotificationPosition),
                &view.position_choices,
                &view.position,
                E::SetPosition,
            )
            .finish(table)),
            row(choice(
                Id::AreaMode,
                t(UiText::NotificationAreaMode),
                &view.area_mode_choices,
                &view.area_mode,
                E::SetAreaMode,
            )
            .finish(table)),
        ],
    ));
    let same_edge = view.position.same_edge(view.status_position);
    groups.push(group(
        lang,
        NotificationGroup::Status,
        UiText::NotificationSectionStatus,
        Some(
            toggle(
                Id::StatusEnabled,
                t(UiText::NotificationEnabled),
                view.status_enabled,
                E::ToggleStatusEnabled,
            )
            .finish(table),
        ),
        vec![
            row(enabled(
                choice(
                    Id::StatusPosition,
                    t(UiText::NotificationStatusPosition),
                    &view.position_choices,
                    &view.status_position,
                    E::SetStatusPosition,
                ),
                view.status_enabled,
            )
            .finish(table)),
            row(visible(
                enabled(
                    choice(
                        Id::StatusAreaMode,
                        t(UiText::NotificationStatusAreaMode),
                        &view.area_mode_choices,
                        &view.status_area_mode,
                        E::SetStatusAreaMode,
                    ),
                    view.status_enabled && !same_edge,
                ),
                !same_edge,
            )
            .finish(table)),
            row(visible(
                note(
                    Id::StatusAreaFollows,
                    t(UiText::NotificationStatusAreaMode),
                    t(UiText::NotificationStatusAreaFollows),
                ),
                same_edge,
            )
            .finish(table)),
        ],
    ));
    groups
}

fn tab(
    id: SettingsTabId,
    label: String,
    selected: bool,
    context: Vec<E>,
    children: Vec<SettingsNode>,
    table: &mut BindingTable,
) -> SettingsTab {
    let mut select = action(
        Id::Tab(id.clone()),
        label.clone(),
        context.last().unwrap().clone(),
    );
    select.context = context[..context.len() - 1].to_vec();
    SettingsTab {
        body: layout(SettingsLayoutId::Tab(id.clone()), children),
        id,
        label,
        selected,
        select: select.finish(table),
    }
}

fn tabs(id: SettingsTabsId, selected: Option<usize>, tabs: Vec<SettingsTab>) -> SettingsNode {
    SettingsNode::Tabs(SettingsTabs {
        id,
        metadata: metadata(""),
        selected,
        tabs,
    })
}

fn systems(controller: &SettingsController, table: &mut BindingTable) -> Vec<SettingsNode> {
    let mut result = Vec::new();
    for (index, vm) in controller.vm.systems().iter().enumerate() {
        let view = vm.view.get();
        let system = SettingsSystemId(view.system_id.to_string());
        let children = view
            .fields
            .iter()
            .map(|field| {
                let mut control = choice(
                    Id::SystemField {
                        system: system.clone(),
                        field: field.id.clone(),
                    },
                    field.label.clone(),
                    &field.choices,
                    &field.selected,
                    |c| E::SetSystemChoice(field.id.as_str().into(), c),
                );
                control.context = vec![E::SelectSystemTab(index)];
                row(control.finish(table))
            })
            .collect();
        result.push(tab(
            SettingsTabId::System(system),
            view.label,
            controller.system_tab_index == Some(index),
            vec![E::SelectSystemTab(index)],
            children,
            table,
        ));
    }
    let mut children = vec![tabs(
        SettingsTabsId::Systems,
        controller.system_tab_index.filter(|i| *i < result.len()),
        result,
    )];
    if controller.vm.systems().is_empty() {
        let label = match resolve_language(controller.language()) {
            AppLanguage::Japanese => "利用可能なシステムがありません",
            _ => "No systems available",
        };
        children.push(row(
            note(Id::NoSystems, String::new(), label.into()).finish(table)
        ));
    }
    children
}

fn binding(
    controller: &SettingsController,
    row_view: &BindingRowView,
    table: &mut BindingTable,
) -> SettingsNode {
    let lang = controller.language();
    row(control(
        Id::Binding(row_view.target.clone()),
        row_view.label.clone(),
        Kind::Binding {
            value: row_view.value.clone(),
            change_label: text(lang, UiText::Change).into(),
            clear_label: text(lang, UiText::Clear).into(),
        },
        ControlBinding::Capture(row_view.target.clone()),
    )
    .finish(table))
}

fn input(controller: &SettingsController, table: &mut BindingTable) -> Vec<SettingsNode> {
    let mut result = Vec::new();
    for (index, vm) in controller.vm.inputs().iter().enumerate() {
        let view = vm.view.get();
        let system = SettingsSystemId(view.system_id.to_string());
        let conflicts = view
            .conflicts
            .iter()
            .enumerate()
            .map(|(index, conflict)| {
                row(note(
                    Id::Conflict {
                        system: system.clone(),
                        index,
                    },
                    String::new(),
                    conflict.message.clone(),
                )
                .finish(table))
            })
            .collect();
        let slots = view
            .slots
            .iter()
            .map(|slot| {
                let mut control = choice(
                    Id::ControllerSlot {
                        system: system.clone(),
                        slot: slot.slot_id,
                    },
                    slot.label.clone(),
                    &slot.choices,
                    &slot.selected_profile_id,
                    |c| E::SetControllerSlot {
                        slot: slot.slot_id,
                        controller_id: c.value,
                    },
                );
                control.context = vec![E::SelectInputTab(index)];
                control.view.metadata.enabled = !slot.occupied_by_other_slot;
                if slot.occupied_by_other_slot {
                    control.view.metadata.help = Some(
                        match resolve_language(controller.language()) {
                            AppLanguage::Japanese => "他のスロットで使用中",
                            _ => "Occupied by another slot",
                        }
                        .into(),
                    );
                }
                row(control.finish(table))
            })
            .collect();
        let sections = view
            .sections
            .iter()
            .enumerate()
            .map(|(section_index, section)| {
                let id = section
                    .rows
                    .iter()
                    .find_map(|r| match &r.target {
                        CaptureTarget::Binding { attachment, .. } => {
                            Some(SettingsTabId::Attachment {
                                system: system.clone(),
                                attachment: attachment.clone(),
                            })
                        }
                        _ => None,
                    })
                    .unwrap_or_else(|| SettingsTabId::EmptyAttachment {
                        system: system.clone(),
                        index: section_index,
                    });
                tab(
                    id,
                    section.label.clone(),
                    controller.input_section == InputPageSection::Attachment(section_index),
                    vec![
                        E::SelectInputTab(index),
                        E::SelectInputSection(InputPageSection::Attachment(section_index)),
                    ],
                    section
                        .rows
                        .iter()
                        .map(|r| binding(controller, r, table))
                        .collect(),
                    table,
                )
            })
            .collect::<Vec<_>>();
        let InputPageSection::Attachment(selected) = controller.input_section;
        let children = vec![
            SettingsNode::Column(layout(
                SettingsLayoutId::Conflicts(system.clone()),
                conflicts,
            )),
            SettingsNode::Group(layout(SettingsLayoutId::Slots(system.clone()), slots)),
            tabs(
                SettingsTabsId::Attachments(system.clone()),
                (selected < sections.len()).then_some(selected),
                sections,
            ),
        ];
        result.push(tab(
            SettingsTabId::Input(system),
            view.label,
            controller.input_tab == Some(InputTab::System(index)),
            vec![E::SelectInputTab(index)],
            children,
            table,
        ));
    }
    let shortcuts = controller.vm.shortcuts.view.get();
    // With no systems, present the only useful tab without mutating controller navigation.
    let selected = match controller.input_tab {
        Some(InputTab::System(index)) => (index < result.len()).then_some(index),
        Some(InputTab::Shortcuts) => Some(result.len()),
        None if result.is_empty() => Some(0),
        None => None,
    };
    result.push(tab(
        SettingsTabId::Shortcuts,
        shortcuts.label,
        selected == Some(result.len()),
        vec![E::SelectInputTab(controller.vm.inputs().len())],
        shortcuts
            .rows
            .iter()
            .map(|r| binding(controller, r, table))
            .collect(),
        table,
    ));
    vec![tabs(SettingsTabsId::Inputs, selected, result)]
}
