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
