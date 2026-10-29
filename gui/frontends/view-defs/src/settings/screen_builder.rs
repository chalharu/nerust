use nerust_gui_viewmodel::settings::dto::{BindingRowView, BindingValueView, ChoiceView};
use nerust_settings_core::i18n::{UiText, resolve_language, text};

use super::*;
use crate::settings::{InputPageSection, InputTab, SettingsEvent as E};

use super::{Builder, SettingsControlId as Id};

/// Single-borrow row helpers: each threads `&mut Builder` once, so call
/// sites never nest mutable borrows.
fn row_toggle(
    b: &mut Builder<'_>,
    id: Id,
    label: String,
    value: bool,
    event: fn(bool) -> E,
) -> Node {
    let key = id.clone();
    let control = draft_toggle(b, id, label, value, event).finish(&mut b.table);
    row(b, &key, control)
}

fn row_choice<T: Clone + Eq>(
    b: &mut Builder<'_>,
    id: Id,
    label: String,
    choices: &[ChoiceView<T>],
    value: &T,
    event: impl Fn(ChoiceView<T>) -> E,
) -> Node {
    let key = id.clone();
    let control = draft_choice(b, id, label, choices, value, event).finish(&mut b.table);
    row(b, &key, control)
}

fn row_choice_enabled<T: Clone + Eq>(
    b: &mut Builder<'_>,
    id: Id,
    label: String,
    choices: &[ChoiceView<T>],
    value: &T,
    event: impl Fn(ChoiceView<T>) -> E,
    on: bool,
) -> Node {
    let key = id.clone();
    let control =
        enabled(draft_choice(b, id, label, choices, value, event), on).finish(&mut b.table);
    row(b, &key, control)
}

fn row_number(b: &mut Builder<'_>, id: Id, label: String, value: u32, event: fn(u32) -> E) -> Node {
    let key = id.clone();
    let control = draft_number(b, id, label, value, event).finish(&mut b.table);
    row(b, &key, control)
}

fn row_number_enabled(
    b: &mut Builder<'_>,
    id: Id,
    label: String,
    value: u32,
    event: fn(u32) -> E,
    on: bool,
) -> Node {
    let key = id.clone();
    let control = enabled(draft_number(b, id, label, value, event), on).finish(&mut b.table);
    row(b, &key, control)
}

fn row_note(b: &mut Builder<'_>, id: Id, label: String, value: String) -> Node {
    let key = id.clone();
    let control = draft_note(b, id, label, value).finish(&mut b.table);
    row(b, &key, control)
}

fn row_note_visible(b: &mut Builder<'_>, id: Id, label: String, value: String, show: bool) -> Node {
    let key = id.clone();
    let control = visible(draft_note(b, id, label, value), show).finish(&mut b.table);
    row(b, &key, control)
}

/// Enabled/visible gates for one row. Bundled so row helpers stay under
/// the argument limit without nesting mutable borrows at call sites.
#[derive(Clone, Copy)]
struct Gate {
    enabled: bool,
    visible: bool,
}

fn row_choice_gated<T: Clone + Eq>(
    b: &mut Builder<'_>,
    id: Id,
    label: String,
    choices: &[ChoiceView<T>],
    value: &T,
    event: impl Fn(ChoiceView<T>) -> E,
    gate: Gate,
) -> Node {
    let key = id.clone();
    let control = visible(
        enabled(draft_choice(b, id, label, choices, value, event), gate.enabled),
        gate.visible,
    )
    .finish(&mut b.table);
    row(b, &key, control)
}

fn draft_toggle(
    b: &mut Builder<'_>,
    id: Id,
    label: String,
    value: bool,
    event: fn(bool) -> E,
) -> Draft {
    b.draft(
        id,
        label,
        ToggleLeaf { value },
        ControlBinding::Toggle(event),
    )
}

fn draft_choice<T: Clone + Eq>(
    b: &mut Builder<'_>,
    id: Id,
    label: String,
    choices: &[ChoiceView<T>],
    value: &T,
    event: impl Fn(ChoiceView<T>) -> E,
) -> Draft {
    b.draft(
        id,
        label,
        ChoiceLeaf {
            choices: choices.iter().map(|c| c.label.clone()).collect(),
            selected: choices.iter().position(|c| &c.value == value),
        },
        ControlBinding::Choice(choices.iter().cloned().map(event).collect()),
    )
}

fn draft_number(
    b: &mut Builder<'_>,
    id: Id,
    label: String,
    value: u32,
    event: fn(u32) -> E,
) -> Draft {
    let (min, max, step, unit) = match id {
        Id::Volume => (0, 100, 1, "%"),
        Id::Latency => (10, 200, 1, "ms"),
        _ => (1000, 30000, 500, "ms"),
    };
    b.draft(
        id,
        label,
        NumberLeaf {
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

fn draft_note(b: &mut Builder<'_>, id: Id, label: String, value: String) -> Draft {
    let accessible_label = if label.is_empty() {
        value.clone()
    } else {
        format!("{label}: {value}")
    };
    let mut result = b.draft(id, label, NoteLeaf { text: value }, ControlBinding::None);
    result.view.metadata.accessible_label = accessible_label;
    result.view.display = ControlDisplay::Note;
    result
}

fn draft_action(b: &mut Builder<'_>, id: Id, label: String, event: E) -> Draft {
    b.draft(id, label, ActionLeaf, ControlBinding::Action(event))
}

fn enabled(mut draft: Draft, enabled: bool) -> Draft {
    draft.view.metadata.enabled = enabled;
    draft
}

fn visible(mut draft: Draft, visible: bool) -> Draft {
    draft.view.metadata.visible = visible;
    draft
}

fn row(b: &mut Builder<'_>, key: &Id, control: Control) -> Node {
    b.row(control, key)
}

pub(super) fn build(
    controller: &SettingsController,
    registry: &mut IdRegistry<StableKey>,
) -> SettingsScreen {
    let mut b = Builder::from_registry(registry);
    let language = controller.language();
    let t = |key| text(language, key).to_string();
    let validation = controller.validation_banner();
    let general_nodes = general(controller, &mut b);
    let input_nodes = input(controller, &mut b);
    let video_nodes = video(controller, &mut b);
    let audio_nodes = audio(controller, &mut b);
    let notification_nodes = notifications(controller, &mut b);
    let system_nodes = systems(controller, &mut b);
    let definitions = [
        (SettingsPage::General, UiText::General, general_nodes),
        (SettingsPage::Input, UiText::Input, input_nodes),
        (SettingsPage::Video, UiText::Video, video_nodes),
        (SettingsPage::Audio, UiText::Audio, audio_nodes),
        (
            SettingsPage::Notifications,
            UiText::Notifications,
            notification_nodes,
        ),
        (SettingsPage::System, UiText::System, system_nodes),
    ];
    let mut cancel = draft_action(&mut b, Id::Cancel, t(UiText::Cancel), E::Cancel);
    cancel.view.display = ControlDisplay::Secondary;
    let mut submit = enabled(
        draft_action(&mut b, Id::Submit, t(UiText::Ok), E::Submit),
        validation.can_submit,
    );
    submit.view.display = ControlDisplay::Primary;
    let table = std::mem::take(&mut b.table);
    let prompt = controller.vm.capture.view.get().prompt.clone();
    let cancel_node = Node::Control(cancel.finish(&mut b.table));
    let submit_node = Node::Control(submit.finish(&mut b.table));
    let mut screen = ControlScreen {
        title: t(UiText::Preferences),
        pages: definitions
            .into_iter()
            .map(|(id, key, children)| {
                let label = t(key);
                let select = b.page_select(id, label.clone());
                let body = b.page_body(id, children);
                Page {
                    id: b.ids.id(StableKey::Page(id)),
                    label,
                    selected: controller.page == id,
                    select,
                    body,
                }
            })
            .collect(),
        message: controller
            .error_message
            .clone()
            .or_else(|| validation.headline.clone()),
        footer: b.footer(vec![cancel_node, submit_node]),
        capture: Capture {
            target: None,
            prompt,
        },
    };
    // Footer controls were finished after the table move; merge them back.
    let mut bindings = table;
    bindings.0.append(&mut b.table.0);
    let capture_target = controller
        .vm
        .capture
        .view
        .get()
        .target
        .as_ref()
        .and_then(|target| {
            bindings
                .0
                .iter()
                .find_map(|(id, binding, _)| match binding {
                    ControlBinding::Capture(candidate) if candidate == target => Some(*id),
                    _ => None,
                })
        });
    screen.capture.target = capture_target;
    SettingsScreen { screen, bindings }
}

fn general(controller: &SettingsController, b: &mut Builder<'_>) -> Vec<Node> {
    let view = controller.vm.general.view.get();
    let t = |key| text(view.language, key).to_string();
    let error = controller.validation_banner().storage;
    // Storage text control needs custom enabled/visible gating.
    let storage_key = Id::StorageDirectory;
    let storage = visible(
        enabled(
            b.draft(
                storage_key.clone(),
                t(UiText::SaveStorageDirectory),
                TextLeaf {
                    // Native edit buffer is controller state, not another settings draft.
                    value: controller.storage_directory_input.clone(),
                    placeholder: String::new(),
                    browse_label: Some(t(UiText::Browse)),
                },
                ControlBinding::Text(E::SetStorageDirectory),
            ),
            view.show_storage_directory,
        ),
        view.show_storage_directory,
    )
    .finish(&mut b.table);
    vec![
        row_choice(
            b,
            Id::Language,
            t(UiText::Language),
            &view.language_choices,
            &view.language,
            E::SetLanguage,
        ),
        row_choice(
            b,
            Id::StoragePolicy,
            t(UiText::SaveStoragePolicy),
            &view.storage_policy_choices,
            &view.storage_policy,
            E::SetStoragePolicy,
        ),
        row(b, &storage_key, storage),
        row_note_visible(
            b,
            Id::StorageError,
            String::new(),
            error.clone().unwrap_or_default(),
            view.show_storage_directory && error.is_some(),
        ),
    ]
}

fn video(controller: &SettingsController, b: &mut Builder<'_>) -> Vec<Node> {
    let view = controller.vm.video.view.get();
    let t = |key| text(controller.language(), key).to_string();
    vec![
        row_toggle(
            b,
            Id::Fullscreen,
            t(UiText::FullscreenDefault),
            view.fullscreen_default,
            E::ToggleFullscreenDefault,
        ),
        row_choice(
            b,
            Id::Scaling,
            t(UiText::Scaling),
            &view.scaling_choices,
            &view.scaling,
            E::SetScaling,
        ),
        row_toggle(b, Id::Vsync, t(UiText::Vsync), view.vsync, E::ToggleVsync),
    ]
}

fn audio(controller: &SettingsController, b: &mut Builder<'_>) -> Vec<Node> {
    let view = controller.vm.audio.view.get();
    let t = |key| text(controller.language(), key).to_string();
    vec![
        row_toggle(b, Id::Mute, t(UiText::Mute), view.muted, E::ToggleMute),
        row_number(
            b,
            Id::Volume,
            t(UiText::MasterVolume),
            view.volume_percent.into(),
            |v| E::SetVolume(v as u8),
        ),
        row_choice(
            b,
            Id::SampleRate,
            t(UiText::SampleRate),
            &view.sample_rate_choices,
            &view.sample_rate,
            E::SetSampleRate,
        ),
        row_number(
            b,
            Id::Latency,
            t(UiText::AudioLatency),
            view.latency_ms.into(),
            |v| E::SetLatency(v as u16),
        ),
    ]
}

fn group(
    b: &mut Builder<'_>,
    language: AppLanguage,
    id: NotificationGroup,
    title: UiText,
    header: Option<Control>,
    children: Vec<Node>,
) -> Node {
    b.group(
        language,
        id,
        text(language, title).into(),
        header,
        children,
        16.0,
    )
}

fn header_toggle(
    b: &mut Builder<'_>,
    id: Id,
    label: String,
    on: bool,
    event: fn(bool) -> E,
) -> Control {
    draft_toggle(b, id, label, on, event).finish(&mut b.table)
}

fn notifications(controller: &SettingsController, b: &mut Builder<'_>) -> Vec<Node> {
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
        let header = Some(header_toggle(
            b,
            enabled_id,
            t(UiText::NotificationEnabled),
            on,
            toggle_event,
        ));
        let timed = vec![
            row_number_enabled(
                b,
                duration_id,
                t(UiText::NotificationDuration),
                duration,
                duration_event,
                on,
            ),
            row_choice_enabled(
                b,
                color_id,
                t(UiText::NotificationColor),
                &view.color_choices,
                &color,
                color_event,
                on,
            ),
        ];
        groups.push(group(b, lang, group_id, title, header, timed));
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
    let error_header = Some(header_toggle(
        b,
        Id::ErrorEnabled,
        t(UiText::NotificationEnabled),
        view.error_enabled,
        E::ToggleErrorEnabled,
    ));
    let error_modal = view.error_modal;
    let error_children = vec![
        row_choice_enabled(
            b,
            Id::ErrorPresentation,
            t(UiText::NotificationError),
            &presentations,
            &error_modal,
            |c| E::ToggleErrorModal(c.value),
            view.error_enabled,
        ),
        row_choice_gated(
            b,
            Id::ErrorColor,
            t(UiText::NotificationColor),
            &view.color_choices,
            &view.error_color,
            E::SetErrorColor,
            Gate {
                enabled: view.error_enabled && !error_modal,
                visible: !error_modal,
            },
        ),
    ];
    groups.push(group(
        b,
        lang,
        NotificationGroup::Error,
        UiText::Error,
        error_header,
        error_children,
    ));
    let display_children = vec![
        row_choice(
            b,
            Id::Position,
            t(UiText::NotificationPosition),
            &view.position_choices,
            &view.position,
            E::SetPosition,
        ),
        row_choice(
            b,
            Id::AreaMode,
            t(UiText::NotificationAreaMode),
            &view.area_mode_choices,
            &view.area_mode,
            E::SetAreaMode,
        ),
    ];
    groups.push(group(
        b,
        lang,
        NotificationGroup::Display,
        UiText::NotificationSectionDisplay,
        None,
        display_children,
    ));
    let same_edge = view.position.same_edge(view.status_position);
    let status_header = Some(header_toggle(
        b,
        Id::StatusEnabled,
        t(UiText::NotificationEnabled),
        view.status_enabled,
        E::ToggleStatusEnabled,
    ));
    let status_enabled = view.status_enabled;
    let status_children = vec![
        row_choice_enabled(
            b,
            Id::StatusPosition,
            t(UiText::NotificationStatusPosition),
            &view.position_choices,
            &view.status_position,
            E::SetStatusPosition,
            status_enabled,
        ),
        row_choice_gated(
            b,
            Id::StatusAreaMode,
            t(UiText::NotificationStatusAreaMode),
            &view.area_mode_choices,
            &view.status_area_mode,
            E::SetStatusAreaMode,
            Gate {
                enabled: status_enabled && !same_edge,
                visible: !same_edge,
            },
        ),
        row_note_visible(
            b,
            Id::StatusAreaFollows,
            t(UiText::NotificationStatusAreaMode),
            t(UiText::NotificationStatusAreaFollows),
            same_edge,
        ),
    ];
    groups.push(group(
        b,
        lang,
        NotificationGroup::Status,
        UiText::NotificationSectionStatus,
        status_header,
        status_children,
    ));
    groups
}

fn tab(
    id: SettingsTabId,
    label: String,
    selected: bool,
    context: Vec<E>,
    children: Vec<Node>,
) -> TabSpec {
    TabSpec {
        id,
        label,
        selected,
        context,
        children,
    }
}

fn tabs(
    b: &mut Builder<'_>,
    id: SettingsTabsId,
    selected: Option<usize>,
    tabs: Vec<TabSpec>,
) -> Node {
    b.tabs(id, selected, tabs)
}

fn systems(controller: &SettingsController, b: &mut Builder<'_>) -> Vec<Node> {
    let mut result = Vec::new();
    for (index, vm) in controller.vm.systems().iter().enumerate() {
        let view = vm.view.get();
        let system = SettingsSystemId(view.system_id.to_string());
        let children = view
            .fields
            .iter()
            .map(|field| {
                let key = Id::SystemField {
                    system: system.clone(),
                    field: field.id.clone(),
                };
                let select_index = index;
                let choice = draft_choice(
                    b,
                    key.clone(),
                    field.label.clone(),
                    &field.choices,
                    &field.selected,
                    move |c| E::SetSystemChoice(field.id.as_str().into(), c),
                );
                let mut choice = choice;
                choice.context = vec![E::SelectSystemTab(select_index)];
                let control = choice.finish(&mut b.table);
                row(b, &key, control)
            })
            .collect();
        result.push(tab(
            SettingsTabId::System(system),
            view.label,
            controller.system_tab_index == Some(index),
            vec![E::SelectSystemTab(index)],
            children,
        ));
    }
    let mut children = vec![tabs(
        b,
        SettingsTabsId::Systems,
        controller.system_tab_index.filter(|i| *i < result.len()),
        result,
    )];
    if controller.vm.systems().is_empty() {
        let label = match resolve_language(controller.language()) {
            AppLanguage::Japanese => "利用可能なシステムがありません",
            _ => "No systems available",
        };
        children.push(row_note(b, Id::NoSystems, String::new(), label.into()));
    }
    children
}

fn binding_row(
    controller: &SettingsController,
    b: &mut Builder<'_>,
    row_view: &BindingRowView,
) -> Node {
    let lang = controller.language();
    let key = Id::Binding(row_view.target.clone());
    let value = match &row_view.value {
        BindingValueView::Unbound(t)
        | BindingValueView::Bound(t)
        | BindingValueView::Capturing(t) => t.clone(),
    };
    let capturing = matches!(row_view.value, BindingValueView::Capturing(_));
    let control = b
        .draft(
            key.clone(),
            row_view.label.clone(),
            BindingLeaf {
                text: value,
                capturing,
                change_label: text(lang, UiText::Change).into(),
                clear_label: text(lang, UiText::Clear).into(),
            },
            ControlBinding::Capture(row_view.target.clone()),
        )
        .finish(&mut b.table);
    row(b, &key, control)
}

fn input(controller: &SettingsController, b: &mut Builder<'_>) -> Vec<Node> {
    let mut result = Vec::new();
    for (index, vm) in controller.vm.inputs().iter().enumerate() {
        let view = vm.view.get();
        let system = SettingsSystemId(view.system_id.to_string());
        let conflicts = view
            .conflicts
            .iter()
            .enumerate()
            .map(|(conflict_index, conflict)| {
                let key = Id::Conflict {
                    system: system.clone(),
                    index: conflict_index,
                };
                let note = draft_note(b, key.clone(), String::new(), conflict.message.clone())
                    .finish(&mut b.table);
                // Conflict rows lack domain identity; give them content keys.
                b.conflict_row(note, &key, &conflict.message)
            })
            .collect();
        let slots = view
            .slots
            .iter()
            .map(|slot| {
                let key = Id::ControllerSlot {
                    system: system.clone(),
                    slot: slot.slot_id,
                };
                let select_index = index;
                let mut choice = draft_choice(
                    b,
                    key.clone(),
                    slot.label.clone(),
                    &slot.choices,
                    &slot.selected_profile_id,
                    move |c| E::SetControllerSlot {
                        slot: slot.slot_id,
                        controller_id: c.value,
                    },
                );
                choice.context = vec![E::SelectInputTab(select_index)];
                choice.view.metadata.enabled = !slot.occupied_by_other_slot;
                if slot.occupied_by_other_slot {
                    choice.view.metadata.help = Some(
                        match resolve_language(controller.language()) {
                            AppLanguage::Japanese => "他のスロットで使用中",
                            _ => "Occupied by another slot",
                        }
                        .into(),
                    );
                }
                let control = choice.finish(&mut b.table);
                row(b, &key, control)
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
                        .map(|r| binding_row(controller, b, r))
                        .collect(),
                )
            })
            .collect::<Vec<_>>();
        let InputPageSection::Attachment(selected) = controller.input_section;
        let children = vec![
            b.column(SettingsLayoutId::Conflicts(system.clone()), conflicts),
            b.column(SettingsLayoutId::Slots(system.clone()), slots),
            tabs(
                b,
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
            .map(|r| binding_row(controller, b, r))
            .collect(),
    ));
    vec![tabs(b, SettingsTabsId::Inputs, selected, result)]
}
