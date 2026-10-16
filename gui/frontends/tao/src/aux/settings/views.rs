//! Settings page builders: one function per dialog page plus
//! the small widget helpers they share. State manipulation stays in
//! [`super::program`]; this module only reads state and builds widgets.

use iced::{
    Alignment, Length, Theme,
    widget::{Column, button, checkbox, column, pick_list, radio, row, slider, text, text_input},
};
use nerust_gui_viewmodel::settings::dto::ChoiceView;
use nerust_settings_core::i18n::{UiText, text as ui_text};
use nerust_settings_core::model::language::AppLanguage;

use super::program::{El, InputPageSection, Message, SettingsAppState, SettingsPage};

impl SettingsAppState {
    pub(super) fn page_content(&self) -> El<'_> {
        match self.page {
            SettingsPage::General => self.general_page(),
            SettingsPage::Input => self.input_page(),
            SettingsPage::Video => self.video_page(),
            SettingsPage::Audio => self.audio_page(),
            SettingsPage::Notifications => self.notifications_page(),
            SettingsPage::System => self.system_page(),
        }
    }

    fn general_page(&self) -> El<'_> {
        let general = self.vm.general.view.get();
        let language = general.language;
        let mut content = column![
            labeled_pick_list(
                ui_text(language, UiText::Language),
                general.language_choices.clone(),
                pick_selected(&general.language_choices, &general.language),
                Message::SetLanguage
            ),
            labeled_pick_list(
                ui_text(language, UiText::SaveStoragePolicy),
                general.storage_policy_choices.clone(),
                pick_selected(&general.storage_policy_choices, &general.storage_policy),
                Message::SetStoragePolicy
            ),
        ]
        .spacing(16);

        if general.show_storage_directory {
            let storage_row = row![
                text(ui_text(language, UiText::SaveStorageDirectory)).width(Length::Fixed(220.0)),
                text_input("", &self.storage_directory_input)
                    .on_input(Message::SetStorageDirectory)
                    .width(Length::Fill),
                button(ui_text(language, UiText::Browse)).on_press(Message::BrowseStorageDirectory),
            ]
            .spacing(12)
            .align_y(Alignment::Center);
            content = content.push(storage_row);
            if let Some(error) = self.storage_error() {
                content = content.push(text(error));
            }
        }

        content.into()
    }

    fn input_page(&self) -> El<'_> {
        let Some(input_tab_index) = self.input_tab_index else {
            return column![text("No systems available").size(14)].into();
        };
        let inputs = self.vm.inputs();
        let Some(input_vm) = inputs.get(input_tab_index) else {
            return column![text("No systems available").size(14)].into();
        };
        let view = input_vm.view.get();

        let mut content: Column<Message, Theme, iced_tiny_skia::Renderer> = column![];
        content = content.push(input_tab_row(inputs, self.input_tab_index));

        // Conflicts
        for conflict in &view.conflicts {
            content = content.push(text(conflict.message.clone()));
        }
        content = push_controller_slots(content, &view.slots);

        let input_section_content = self.input_nav_section(&view);
        content.push(input_section_content).spacing(16).into()
    }

    /// Navigation radios plus the selected binding section.
    fn input_nav_section(
        &self,
        view: &nerust_gui_viewmodel::settings::dto::InputTabView,
    ) -> El<'_> {
        // Build navigation tabs as owned data to avoid lifetime issues
        let section_labels: Vec<String> = view.sections.iter().map(|s| s.label.clone()).collect();
        let mut navigation = row![].spacing(16).align_y(Alignment::Center);
        for (index, label) in section_labels.iter().enumerate() {
            navigation = navigation.push(radio(
                label.clone(),
                InputPageSection::Attachment(index),
                Some(self.input_section),
                Message::SelectInputSection,
            ));
        }
        // Content section: clone all data upfront to avoid lifetime issues
        let input_section_content = match self.input_section {
            InputPageSection::Attachment(index) => {
                if let Some(section) = view.sections.get(index) {
                    let rows: Vec<nerust_gui_viewmodel::settings::dto::BindingRowView> =
                        section.rows.clone();
                    let title = section.label.clone();
                    self.binding_section(title, rows)
                } else {
                    column![text("")].into()
                }
            }
        };
        column![navigation, input_section_content].into()
    }

    fn binding_section(
        &self,
        title: String,
        rows: Vec<nerust_gui_viewmodel::settings::dto::BindingRowView>,
    ) -> El<'_> {
        let language = self.language();
        let mut content: Column<Message, Theme, iced_tiny_skia::Renderer> = column![text(title)];
        for row in rows {
            let binding_label = match &row.value {
                nerust_gui_viewmodel::settings::dto::BindingValueView::Bound(l) => l.clone(),
                nerust_gui_viewmodel::settings::dto::BindingValueView::Unbound(l) => l.clone(),
                nerust_gui_viewmodel::settings::dto::BindingValueView::Capturing(l) => l.clone(),
            };
            let target = row.target.clone();
            content = content.push(
                row![
                    text(row.label.clone()).width(Length::Fixed(180.0)),
                    text(binding_label).width(Length::Fill),
                    button(ui_text(language, UiText::Change))
                        .on_press(Message::StartCapture(target.clone())),
                    button(ui_text(language, UiText::Clear))
                        .on_press(Message::ClearCapture(target)),
                ]
                .spacing(12)
                .width(Length::Fill)
                .align_y(Alignment::Center),
            );
        }
        content.spacing(8).into()
    }

    fn video_page(&self) -> El<'_> {
        let video = self.vm.video.view.get();
        let language = self.language();
        column![
            checkbox(video.fullscreen_default)
                .label(ui_text(language, UiText::FullscreenDefault))
                .on_toggle(Message::ToggleFullscreenDefault),
            labeled_pick_list(
                ui_text(language, UiText::Scaling),
                video.scaling_choices.clone(),
                pick_selected(&video.scaling_choices, &video.scaling),
                Message::SetScaling
            ),
            checkbox(video.vsync)
                .label(ui_text(language, UiText::Vsync))
                .on_toggle(Message::ToggleVsync),
        ]
        .spacing(16)
        .into()
    }

    fn audio_page(&self) -> El<'_> {
        let audio = self.vm.audio.view.get();
        let language = self.language();
        column![
            checkbox(audio.muted)
                .label(ui_text(language, UiText::Mute))
                .on_toggle(Message::ToggleMute),
            labeled_slider(
                ui_text(language, UiText::MasterVolume),
                format!("{}%", audio.volume_percent),
                slider(0..=100, audio.volume_percent, Message::SetVolume)
            ),
            labeled_pick_list(
                ui_text(language, UiText::SampleRate),
                audio.sample_rate_choices.clone(),
                pick_selected(&audio.sample_rate_choices, &audio.sample_rate),
                Message::SetSampleRate
            ),
            labeled_slider(
                ui_text(language, UiText::AudioLatency),
                format!("{} ms", audio.latency_ms),
                slider(10..=200, audio.latency_ms, Message::SetLatency)
            ),
        ]
        .spacing(16)
        .into()
    }

    fn notifications_page(&self) -> El<'_> {
        use nerust_settings_core::model::local::{
            NotificationAreaMode, NotificationColor, NotificationPosition,
        };
        let view = self.vm.notifications.view.get();
        let language = self.language();
        let colors = vec![
            ChoiceView {
                value: NotificationColor::White,
                label: ui_text(language, UiText::ColorWhite).to_string(),
            },
            ChoiceView {
                value: NotificationColor::Yellow,
                label: ui_text(language, UiText::ColorYellow).to_string(),
            },
            ChoiceView {
                value: NotificationColor::Red,
                label: ui_text(language, UiText::ColorRed).to_string(),
            },
            ChoiceView {
                value: NotificationColor::Green,
                label: ui_text(language, UiText::ColorGreen).to_string(),
            },
            ChoiceView {
                value: NotificationColor::Cyan,
                label: ui_text(language, UiText::ColorCyan).to_string(),
            },
        ];
        let positions = vec![
            ChoiceView {
                value: NotificationPosition::TopLeft,
                label: ui_text(language, UiText::PosTopLeft).to_string(),
            },
            ChoiceView {
                value: NotificationPosition::TopRight,
                label: ui_text(language, UiText::PosTopRight).to_string(),
            },
            ChoiceView {
                value: NotificationPosition::BottomLeft,
                label: ui_text(language, UiText::PosBottomLeft).to_string(),
            },
            ChoiceView {
                value: NotificationPosition::BottomRight,
                label: ui_text(language, UiText::PosBottomRight).to_string(),
            },
        ];
        let areas = vec![
            ChoiceView {
                value: NotificationAreaMode::Overlay,
                label: ui_text(language, UiText::AreaOverlay).to_string(),
            },
            ChoiceView {
                value: NotificationAreaMode::Strip,
                label: ui_text(language, UiText::AreaStrip).to_string(),
            },
        ];
        // One section per level plus display/status groups, so each
        // control's scope reads off its header (Apple HIG grouping).
        // Error presentation is an exclusive pair: modal dialog XOR
        // on-screen display, matching the delivery contract.
        let info = section_toggled(
            ui_text(language, UiText::NotificationInfo),
            checkbox(view.info_enabled)
                .label(ui_text(language, UiText::NotificationEnabled))
                .on_toggle(Message::ToggleInfoEnabled)
                .into(),
            [
                labeled_slider(
                    ui_text(language, UiText::NotificationDuration),
                    format!("{} ms", view.info_duration_ms),
                    slider(
                        1000..=30000,
                        view.info_duration_ms,
                        Message::SetInfoDuration,
                    ),
                ),
                labeled_pick_list(
                    ui_text(language, UiText::NotificationColor),
                    colors.clone(),
                    pick_selected(&colors, &view.info_color),
                    Message::SetInfoColor,
                ),
            ],
        );
        let warning = section_toggled(
            ui_text(language, UiText::NotificationWarning),
            checkbox(view.warning_enabled)
                .label(ui_text(language, UiText::NotificationEnabled))
                .on_toggle(Message::ToggleWarningEnabled)
                .into(),
            [
                labeled_slider(
                    ui_text(language, UiText::NotificationDuration),
                    format!("{} ms", view.warning_duration_ms),
                    slider(
                        1000..=30000,
                        view.warning_duration_ms,
                        Message::SetWarningDuration,
                    ),
                ),
                labeled_pick_list(
                    ui_text(language, UiText::NotificationColor),
                    colors.clone(),
                    pick_selected(&colors, &view.warning_color),
                    Message::SetWarningColor,
                ),
            ],
        );
        let error = section_toggled(
            ui_text(language, UiText::Error),
            checkbox(view.error_enabled)
                .label(ui_text(language, UiText::NotificationEnabled))
                .on_toggle(Message::ToggleErrorEnabled)
                .into(),
            [
                row![
                    radio(
                        ui_text(language, UiText::NotificationErrorModal).to_string(),
                        true,
                        Some(view.error_modal),
                        Message::ToggleErrorModal
                    ),
                    radio(
                        ui_text(language, UiText::NotificationErrorOnScreen).to_string(),
                        false,
                        Some(view.error_modal),
                        Message::ToggleErrorModal
                    ),
                ]
                .spacing(16)
                .into(),
                labeled_pick_list(
                    ui_text(language, UiText::NotificationColor),
                    colors.clone(),
                    pick_selected(&colors, &view.error_color),
                    Message::SetErrorColor,
                ),
            ],
        );
        let display = section(
            ui_text(language, UiText::NotificationSectionDisplay),
            [
                labeled_pick_list(
                    ui_text(language, UiText::NotificationPosition),
                    positions.clone(),
                    pick_selected(&positions, &view.position),
                    Message::SetPosition,
                ),
                labeled_pick_list(
                    ui_text(language, UiText::NotificationAreaMode),
                    areas.clone(),
                    pick_selected(&areas, &view.area_mode),
                    Message::SetAreaMode,
                ),
            ],
        );
        let status = section_toggled(
            ui_text(language, UiText::NotificationSectionStatus),
            checkbox(view.status_enabled)
                .label(ui_text(language, UiText::NotificationEnabled))
                .on_toggle(Message::ToggleStatusEnabled)
                .into(),
            [labeled_pick_list(
                ui_text(language, UiText::NotificationStatusPosition),
                positions.clone(),
                pick_selected(&positions, &view.status_position),
                Message::SetStatusPosition,
            )],
        );
        column![info, warning, error, display, status]
            .spacing(24)
            .into()
    }

    fn system_page(&self) -> El<'_> {
        let system_tab_index = self.system_tab_index;
        let Some(system_tab_index) = system_tab_index else {
            return column![text("No systems available").size(14)].into();
        };
        let systems = self.vm.systems();
        let Some(system_vm) = systems.get(system_tab_index) else {
            return column![text("No systems available").size(14)].into();
        };
        let view = system_vm.view.get();

        let mut content = column![];

        // Tab buttons
        let names = systems
            .iter()
            .map(|vm| vm.display_name().to_string())
            .collect();
        let tab_row = tab_row(self.system_tab_index, names, Message::SelectSystemTab);
        content = content.push(tab_row);

        // Fields
        for field in &view.fields {
            let choices: Vec<
                nerust_gui_viewmodel::settings::dto::ChoiceView<
                    nerust_core_traits::factory::descriptor::SystemSettingsChoiceId,
                >,
            > = field.choices.clone();
            let selected = choices.iter().find(|c| c.value == field.selected).cloned();
            let label = field.label.clone();
            let field_id_str = field.id.0.to_string();
            content = content.push(labeled_pick_list(
                &label,
                choices,
                selected,
                move |choice: nerust_gui_viewmodel::settings::dto::ChoiceView<
                    nerust_core_traits::factory::descriptor::SystemSettingsChoiceId,
                >| { Message::SetSystemChoice(field_id_str.clone(), choice) },
            ));
        }
        content.spacing(16).into()
    }
}

// ---------------------------------------------------------------------------
// Helper functions
// ---------------------------------------------------------------------------

/// System tab buttons across the registered cores.
fn input_tab_row(
    inputs: &[nerust_gui_viewmodel::settings::InputSettingsViewModel],
    selected: Option<usize>,
) -> El<'static> {
    let names = inputs
        .iter()
        .map(|vm| vm.display_name().to_string())
        .collect();
    tab_row(selected, names, Message::SelectInputTab)
}

/// Tab-button row from owned labels; children are all owned.
fn tab_row(
    selected: Option<usize>,
    names: Vec<String>,
    select: impl Fn(usize) -> Message,
) -> El<'static> {
    row(names.iter().enumerate().map(|(i, name)| {
        let btn_text = text(name.clone()).size(14);
        if Some(i) == selected {
            button(btn_text).style(button::primary).into()
        } else {
            button(btn_text).on_press(select(i)).into()
        }
    }))
    .spacing(4)
    .into()
}

/// Controller-slot pick lists appended to the page column.
/// Slot data is cloned into widgets, so the slot borrow never
/// escapes into the returned column (separate lifetimes).
fn push_controller_slots<'a>(
    mut content: Column<'a, Message, Theme, iced_tiny_skia::Renderer>,
    slots: &[nerust_gui_viewmodel::settings::dto::ControllerSlotView],
) -> Column<'a, Message, Theme, iced_tiny_skia::Renderer> {
    for slot in slots {
        if slot.occupied_by_other_slot {
            content = content.push(text(format!("{} — (occupied)", slot.label)));
            continue;
        }
        let current = slot.selected_profile_id.as_ref().and_then(|id| {
            slot.choices
                .iter()
                .find(|c| c.value.as_deref() == Some(id.as_str()))
                .cloned()
        });
        let slot_id = slot.slot_id;
        let pick = pick_list(
            slot.choices.clone(),
            current,
            move |choice: ChoiceView<Option<String>>| Message::SetControllerSlot {
                slot: slot_id,
                controller_id: choice.value,
            },
        );
        content = content.push(text(slot.label.clone())).push(pick);
    }
    content
}

/// Page-selector radio shared by the page headers.
pub(super) fn page_radio(
    language: AppLanguage,
    label: UiText,
    value: SettingsPage,
    selected: SettingsPage,
) -> El<'static> {
    radio(
        ui_text(language, label),
        value,
        Some(selected),
        Message::SelectPage,
    )
    .into()
}

/// Titled settings group: 16 bold header, 4 below / 24 between.
/// Header uses Bold (DejaVu has no Semibold).
fn section<'a>(title: &'a str, rows: impl IntoIterator<Item = El<'a>>) -> El<'a> {
    section_inner(header_text(title), None, rows)
}

/// Section whose master enable sits on the header row (MS
/// SettingsExpander pattern: title plus primary action on one row).
/// Screen readers meet title then toggle in order, so the generic
/// "Enabled" label keeps its level context without visual duplication.
fn section_toggled<'a>(
    title: &'a str,
    toggle: El<'a>,
    rows: impl IntoIterator<Item = El<'a>>,
) -> El<'a> {
    section_inner(header_text(title), Some(toggle), rows)
}

fn header_text(title: &str) -> El<'static> {
    text(title.to_string())
        .size(16)
        .font(iced::Font {
            weight: iced::font::Weight::Bold,
            ..iced::Font::default()
        })
        .into()
}

fn section_inner<'a>(
    header: El<'a>,
    toggle: Option<El<'a>>,
    rows: impl IntoIterator<Item = El<'a>>,
) -> El<'a> {
    let header_row: El<'a> = match toggle {
        Some(toggle) => row![
            header,
            iced::widget::Space::new().width(Length::Fill),
            toggle
        ]
        .align_y(Alignment::Center)
        .into(),
        None => header,
    };
    let mut items = column![].spacing(8);
    for row in rows {
        items = items.push(row);
    }
    column![header_row, items].spacing(4).into()
}

fn labeled_pick_list<'a, T: Clone + PartialEq + Eq + 'static>(
    label: &str,
    options: Vec<ChoiceView<T>>,
    selected: Option<ChoiceView<T>>,
    on_select: impl Fn(ChoiceView<T>) -> Message + 'a,
) -> El<'a> {
    row![
        text(label.to_string()).width(Length::Fixed(220.0)),
        // Stretch to the pane: Shrink rows strand the fixed window
        // (General shows two short rows in a 960px frame otherwise).
        pick_list(options, selected, on_select).width(Length::Fill)
    ]
    .spacing(12)
    .align_y(Alignment::Center)
    .into()
}

fn labeled_slider<'a>(label: &'static str, value: String, slider: impl Into<El<'a>>) -> El<'a> {
    row![
        text(label).width(Length::Fixed(220.0)),
        slider.into(),
        text(value).width(Length::Fixed(72.0)),
    ]
    .spacing(12)
    .align_y(Alignment::Center)
    .into()
}

fn pick_selected<T: Clone + PartialEq + Eq>(
    options: &[ChoiceView<T>],
    value: &T,
) -> Option<ChoiceView<T>> {
    options.iter().find(|c| &c.value == value).cloned()
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex, atomic::AtomicBool};

    use nerust_core_traits::audio::AudioBackendRegistry;
    use nerust_gui_runtime::settings::SettingsSnapshot;
    use nerust_gui_shell::registry::SystemRegistry;
    use nerust_settings_core::model::{
        app_state::DesktopAppState, local::HostBackendLocalSettings, shared::DesktopSharedSettings,
    };

    use super::super::program::SettingsAppState;
    use super::*;

    fn empty_state() -> SettingsAppState {
        let snapshot = SettingsSnapshot {
            shared: DesktopSharedSettings::default(),
            local: HostBackendLocalSettings::default(),
            app_state: DesktopAppState::default(),
        };
        SettingsAppState::new_with_shared(
            &snapshot,
            Arc::new(SystemRegistry::new(Vec::new())),
            Arc::new(AudioBackendRegistry::new()),
            Arc::new(AtomicBool::new(false)),
            Arc::new(Mutex::new(None)),
            Arc::new(AtomicBool::new(false)),
        )
    }

    #[test]
    fn page_builders_construct_on_empty_registry() {
        let state = empty_state();
        let _ = state.page_content();
        let _ = state.general_page();
        let _ = state.input_page();
        let _ = state.video_page();
        let _ = state.audio_page();
        let _ = state.system_page();
        let _ = state.binding_section("Title".to_string(), Vec::new());
    }

    #[test]
    fn page_content_dispatches_every_page() {
        let mut state = empty_state();
        for page in [
            SettingsPage::General,
            SettingsPage::Input,
            SettingsPage::Video,
            SettingsPage::Audio,
            SettingsPage::Notifications,
            SettingsPage::System,
        ] {
            state.page = page;
            let _ = state.page_content();
        }
        let _ = tab_row(Some(0), vec!["A".to_string()], |_| {
            Message::SelectInputTab(0)
        });
    }

    #[test]
    fn tab_and_slot_helpers_construct() {
        let state = empty_state();
        let inputs = state.vm.inputs();
        let _ = input_tab_row(inputs, None);
        let _ = tab_row(None, vec!["A".to_string()], |_| {
            crate::aux::settings::program::Message::SelectInputTab(0)
        });
        let _ = push_controller_slots(iced::widget::column![], &[]);
    }
}
