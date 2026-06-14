use super::bridge::SettingsChildBridge;
use iced::alignment::Alignment;
use iced::event::{self, Status};
use iced::keyboard::key::{Code, Physical};
use iced::widget::{
    button, checkbox, column, container, pick_list, radio, row, scrollable, slider, text,
    text_input,
};
use iced::{Element, Event, Font, Length, Subscription, Task, Theme};
use nerust_gui_runtime::settings::SettingsSnapshot;
use nerust_gui_runtime::settings::apply::validate_shared_settings;
use nerust_gui_settings::input::KeyboardKey;
use nerust_gui_settings::language::AppLanguage;
use nerust_gui_settings::local::ScalingMode;
use nerust_gui_settings::shared::StoragePolicy;
use nerust_gui_shell::descriptor::{
    SystemSettingsChoiceId, SystemSettingsFieldModel, apply_default_system_settings_choice,
    default_input_topology_descriptor, default_system_settings_page_model,
};
use nerust_gui_shell::settings::bindings::conflicting_keys;
use nerust_gui_shell::settings::bindings::descriptors::{
    keyboard_binding_sections, shortcut_descriptors,
};
use nerust_gui_shell::settings::bindings::keys::keyboard_key_label;
use nerust_gui_shell::settings::editor::{
    CaptureTarget, apply_capture_target, current_binding_label,
};
use nerust_gui_shell::settings::i18n::{UiText, text as ui_text};
use nerust_input_schema::InputTopologyDescriptor;
use rfd::FileDialog;
use std::fmt;
use std::sync::{Arc, Mutex};

#[derive(Debug, Clone, PartialEq, Eq)]
struct Choice<T: Clone + Eq> {
    value: T,
    label: String,
}

impl<T: Clone + Eq> fmt::Display for Choice<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.label)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SettingsPage {
    General,
    Input,
    Video,
    Audio,
    System,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InputPageSection {
    Attachment(usize),
    Shortcuts,
}

#[derive(Debug, Clone)]
enum Message {
    SelectPage(SettingsPage),
    SelectInputSection(InputPageSection),
    SetLanguage(Choice<AppLanguage>),
    SetStoragePolicy(Choice<StoragePolicy>),
    SetStorageDirectory(String),
    BrowseStorageDirectory,
    ToggleFullscreenDefault(bool),
    SetScaling(Choice<ScalingMode>),
    ToggleVsync(bool),
    ToggleMute(bool),
    SetVolume(u8),
    SetSampleRate(Choice<u32>),
    SetLatency(u16),
    SetSystemChoice(String, Choice<String>),
    StartCapture(CaptureTarget),
    ClearCapture(CaptureTarget),
    CaptureKey(KeyboardKey),
    CloseRequested,
    Submit,
    ApplyFinished(Result<(), String>),
    Cancel,
}

struct SettingsApp {
    bridge: Arc<
        Mutex<
            SettingsChildBridge<
                std::io::BufReader<std::io::Stdin>,
                std::io::BufWriter<std::io::Stdout>,
            >,
        >,
    >,
    draft: SettingsSnapshot,
    page: SettingsPage,
    input_section: InputPageSection,
    capture_target: Option<CaptureTarget>,
    storage_directory_input: String,
    error_message: Option<String>,
    submitting: bool,
}

pub(super) fn run(
    snapshot: SettingsSnapshot,
    bridge: SettingsChildBridge<
        std::io::BufReader<std::io::Stdin>,
        std::io::BufWriter<std::io::Stdout>,
    >,
) -> Result<(), String> {
    let bridge = Arc::new(Mutex::new(bridge));
    iced::application(
        move || SettingsApp::new(snapshot.clone(), bridge.clone()),
        update,
        view,
    )
    .title(title)
    .theme(theme)
    .default_font(default_font())
    .subscription(subscription)
    .window(iced::window::Settings {
        size: iced::Size::new(960.0, 720.0),
        resizable: true,
        ..Default::default()
    })
    .run()
    .map_err(|error| format!("settings UI failed: {error}"))
}

fn title(state: &SettingsApp) -> String {
    ui_text(state.language(), UiText::Preferences).into()
}

fn theme(_state: &SettingsApp) -> Theme {
    Theme::Dark
}

fn subscription(state: &SettingsApp) -> Subscription<Message> {
    let close_events = event::listen_with(close_requested_event);
    if state.capture_target.is_some() {
        Subscription::batch([close_events, event::listen_with(capture_key_event)])
    } else {
        close_events
    }
}

fn capture_key_event(event: Event, _status: Status, _window: iced::window::Id) -> Option<Message> {
    match event {
        Event::Keyboard(iced::keyboard::Event::KeyPressed {
            physical_key,
            repeat,
            ..
        }) if !repeat => keyboard_key_from_physical(physical_key).map(Message::CaptureKey),
        _ => None,
    }
}

fn close_requested_event(
    event: Event,
    _status: Status,
    _window: iced::window::Id,
) -> Option<Message> {
    match event {
        Event::Window(iced::window::Event::CloseRequested) => Some(Message::CloseRequested),
        _ => None,
    }
}

fn can_submit_settings(state: &SettingsApp) -> bool {
    !state.submitting && state.validation_errors().is_empty()
}

fn submit_draft(state: &mut SettingsApp) -> Task<Message> {
    if !can_submit_settings(state) {
        return Task::none();
    }
    state.submitting = true;
    let bridge = state.bridge.clone();
    let draft = state.draft.clone();
    Task::perform(
        async move {
            bridge
                .lock()
                .map_err(|_| "settings helper bridge lock was poisoned".to_string())
                .and_then(|mut bridge| bridge.apply_settings(&draft))
        },
        Message::ApplyFinished,
    )
}

fn update(state: &mut SettingsApp, message: Message) -> Task<Message> {
    state.error_message = None;
    match message {
        Message::SelectPage(page) => state.page = page,
        Message::SelectInputSection(section) => state.input_section = section,
        Message::SetLanguage(choice) => state.draft.shared.general.language = choice.value,
        Message::SetStoragePolicy(choice) => {
            state.draft.shared.persistence.storage_policy = choice.value;
        }
        Message::SetStorageDirectory(value) => {
            state.storage_directory_input = value;
            state.draft.shared.persistence.storage_directory =
                (!state.storage_directory_input.is_empty())
                    .then(|| state.storage_directory_input.clone().into());
        }
        Message::BrowseStorageDirectory => {
            if let Some(path) = FileDialog::new()
                .set_title(ui_text(state.language(), UiText::SaveStorageDirectory))
                .pick_folder()
            {
                let path = path.to_string_lossy().to_string();
                state.storage_directory_input = path.clone();
                state.draft.shared.persistence.storage_directory = Some(path.into());
            }
        }
        Message::ToggleFullscreenDefault(value) => {
            state.draft.local.video.window.fullscreen_default = value;
        }
        Message::SetScaling(choice) => state.draft.local.video.window.scaling = choice.value,
        Message::ToggleVsync(value) => state.draft.local.video.presentation.vsync = value,
        Message::ToggleMute(value) => state.draft.local.audio.muted = value,
        Message::SetVolume(value) => state.draft.local.audio.master_volume_percent = value,
        Message::SetSampleRate(choice) => state.draft.local.audio.sample_rate = choice.value,
        Message::SetLatency(value) => state.draft.local.audio.latency_ms = value,
        Message::SetSystemChoice(field, choice) => {
            let _ = apply_default_system_settings_choice(
                &mut state.draft,
                &nerust_gui_shell::descriptor::SystemSettingsFieldId(field.into()),
                &SystemSettingsChoiceId(choice.value.into()),
            );
        }
        Message::StartCapture(target) => state.capture_target = Some(target),
        Message::ClearCapture(target) => {
            apply_capture_target(&mut state.draft, &target, None);
            state.capture_target = None;
        }
        Message::CaptureKey(key) => {
            if let Some(target) = state.capture_target.take() {
                apply_capture_target(&mut state.draft, &target, Some(key));
            }
        }
        Message::CloseRequested | Message::Submit => return submit_draft(state),
        Message::ApplyFinished(result) => {
            state.submitting = false;
            match result {
                Ok(()) => return iced::exit(),
                Err(error) => state.error_message = Some(error),
            }
        }
        Message::Cancel => return iced::exit(),
    }
    Task::none()
}

fn view(state: &SettingsApp) -> Element<'_, Message> {
    let language = state.language();
    let validation_errors = state.validation_errors();
    let can_submit = can_submit_settings(state);

    let sidebar = column![
        page_radio(language, UiText::General, SettingsPage::General, state.page),
        page_radio(language, UiText::Input, SettingsPage::Input, state.page),
        page_radio(language, UiText::Video, SettingsPage::Video, state.page),
        page_radio(language, UiText::Audio, SettingsPage::Audio, state.page),
        page_radio(language, UiText::System, SettingsPage::System, state.page),
    ]
    .spacing(10)
    .width(Length::Shrink);

    let content = scrollable(
        container(state.page_content())
            .padding(12)
            .width(Length::Fill),
    )
    .width(Length::Fill)
    .height(Length::Fill);

    let mut root = column![
        row![
            container(sidebar).padding(12).width(Length::Fixed(180.0)),
            content
        ]
        .spacing(16)
        .width(Length::Fill)
        .height(Length::Fill)
    ]
    .spacing(12)
    .padding(16)
    .height(Length::Fill);

    if let Some(error_message) = state.error_message.as_ref() {
        root = root.push(text(error_message.clone()));
    } else if let Some(first_error) = validation_errors.first() {
        root = root.push(text(first_error.clone()));
    }

    let buttons = row![
        button(ui_text(language, UiText::Cancel)).on_press(Message::Cancel),
        button(ui_text(language, UiText::Ok)).on_press_maybe(can_submit.then_some(Message::Submit)),
    ]
    .spacing(12)
    .align_y(Alignment::Center);

    root.push(container(buttons).width(Length::Fill)).into()
}

impl SettingsApp {
    fn new(
        snapshot: SettingsSnapshot,
        bridge: Arc<
            Mutex<
                SettingsChildBridge<
                    std::io::BufReader<std::io::Stdin>,
                    std::io::BufWriter<std::io::Stdout>,
                >,
            >,
        >,
    ) -> Self {
        let storage_directory_input = snapshot
            .shared
            .persistence
            .storage_directory
            .as_ref()
            .map(|path| path.to_string_lossy().to_string())
            .unwrap_or_default();
        Self {
            bridge,
            draft: snapshot,
            page: SettingsPage::General,
            input_section: InputPageSection::Attachment(0),
            capture_target: None,
            storage_directory_input,
            error_message: None,
            submitting: false,
        }
    }

    fn language(&self) -> AppLanguage {
        self.draft.shared.general.language
    }

    fn validation_errors(&self) -> Vec<String> {
        let mut errors = Vec::new();
        if let Err(error) = validate_shared_settings(&self.draft.shared) {
            errors.push(error.to_string());
        }
        for (key, labels) in conflicting_keys(&self.draft.shared, &input_topology()) {
            errors.push(format!(
                "{}: {}",
                keyboard_key_label(key),
                labels.join(", ")
            ));
        }
        errors
    }

    fn storage_error(&self) -> Option<String> {
        validate_shared_settings(&self.draft.shared)
            .err()
            .map(|error| error.to_string())
    }

    fn input_conflict(&self) -> Option<String> {
        let (key, labels) = conflicting_keys(&self.draft.shared, &input_topology())
            .into_iter()
            .next()?;
        Some(format!(
            "{}: {}",
            keyboard_key_label(key),
            labels.join(", ")
        ))
    }

    fn page_content(&self) -> Element<'_, Message> {
        match self.page {
            SettingsPage::General => self.general_page(),
            SettingsPage::Input => self.input_page(),
            SettingsPage::Video => self.video_page(),
            SettingsPage::Audio => self.audio_page(),
            SettingsPage::System => self.system_page(),
        }
    }

    fn general_page(&self) -> Element<'_, Message> {
        let language = self.language();
        let mut content = column![
            labeled_pick_list(
                ui_text(language, UiText::Language),
                language_options(language),
                selected_choice(
                    self.draft.shared.general.language,
                    language_options(language)
                ),
                Message::SetLanguage
            ),
            labeled_pick_list(
                ui_text(language, UiText::SaveStoragePolicy),
                storage_policy_options(language),
                selected_choice(
                    self.draft.shared.persistence.storage_policy,
                    storage_policy_options(language)
                ),
                Message::SetStoragePolicy
            ),
        ]
        .spacing(16);

        if matches!(
            self.draft.shared.persistence.storage_policy,
            StoragePolicy::CustomDirectory
        ) {
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

    fn input_page(&self) -> Element<'_, Message> {
        let language = self.language();
        let mut content = column![];
        if let Some(conflict) = self.input_conflict() {
            content = content.push(text(conflict));
        }

        let sections = keyboard_binding_sections(&input_topology());
        let mut navigation = row![].spacing(16).align_y(Alignment::Center);
        for (index, section) in sections.iter().enumerate() {
            navigation = navigation.push(input_section_radio_label(
                section.attachment_label,
                InputPageSection::Attachment(index),
                self.input_section,
            ));
        }
        navigation = navigation.push(input_section_radio(
            language,
            UiText::Shortcuts,
            InputPageSection::Shortcuts,
            self.input_section,
        ));

        let section = match self.input_section {
            InputPageSection::Attachment(index) => sections
                .get(index)
                .map(|section| {
                    let rows = section
                        .bindings
                        .clone()
                        .into_iter()
                        .map(|descriptor| {
                            (
                                descriptor.control_label,
                                CaptureTarget::Binding {
                                    system: descriptor.system,
                                    attachment: descriptor.attachment.as_str().to_string(),
                                    control: descriptor.control.as_str().to_string(),
                                },
                            )
                        })
                        .collect::<Vec<_>>();
                    self.input_section(section.attachment_label, rows.into_iter())
                })
                .unwrap_or_else(|| self.input_section("", std::iter::empty())),
            InputPageSection::Shortcuts => self.input_section(
                ui_text(language, UiText::Shortcuts),
                shortcut_descriptors().iter().map(|descriptor| {
                    (descriptor.label, CaptureTarget::Shortcut(descriptor.action))
                }),
            ),
        };

        content.push(navigation).push(section).spacing(16).into()
    }

    fn input_section<'a>(
        &'a self,
        title: &'static str,
        rows: impl Iterator<Item = (&'static str, CaptureTarget)> + 'a,
    ) -> Element<'a, Message> {
        let language = self.language();
        let mut content = column![text(title)];
        for (label, target) in rows {
            let binding_label = if self.capture_target.as_ref() == Some(&target) {
                ui_text(language, UiText::CapturePrompt)
            } else {
                current_binding_label(&self.draft, &target)
                    .unwrap_or(ui_text(language, UiText::Unbound))
            };
            content = content.push(
                row![
                    text(label).width(Length::Fixed(180.0)),
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

    fn video_page(&self) -> Element<'_, Message> {
        let language = self.language();
        column![
            checkbox(self.draft.local.video.window.fullscreen_default)
                .label(ui_text(language, UiText::FullscreenDefault))
                .on_toggle(Message::ToggleFullscreenDefault),
            labeled_pick_list(
                ui_text(language, UiText::Scaling),
                scaling_options(language),
                selected_choice(
                    self.draft.local.video.window.scaling,
                    scaling_options(language)
                ),
                Message::SetScaling
            ),
            checkbox(self.draft.local.video.presentation.vsync)
                .label(ui_text(language, UiText::Vsync))
                .on_toggle(Message::ToggleVsync),
        ]
        .spacing(16)
        .into()
    }

    fn audio_page(&self) -> Element<'_, Message> {
        let language = self.language();
        column![
            checkbox(self.draft.local.audio.muted)
                .label(ui_text(language, UiText::Mute))
                .on_toggle(Message::ToggleMute),
            labeled_slider(
                ui_text(language, UiText::MasterVolume),
                format!("{}%", self.draft.local.audio.master_volume_percent),
                slider(
                    0..=100,
                    self.draft.local.audio.master_volume_percent,
                    Message::SetVolume
                )
            ),
            labeled_pick_list(
                ui_text(language, UiText::SampleRate),
                sample_rate_options(),
                selected_choice(self.draft.local.audio.sample_rate, sample_rate_options()),
                Message::SetSampleRate
            ),
            labeled_slider(
                ui_text(language, UiText::AudioLatency),
                format!("{} ms", self.draft.local.audio.latency_ms),
                slider(
                    10..=200,
                    self.draft.local.audio.latency_ms,
                    Message::SetLatency
                )
            ),
        ]
        .spacing(16)
        .into()
    }

    fn system_page(&self) -> Element<'_, Message> {
        let model = default_system_settings_page_model(&self.draft);
        let mut content = column![];
        for field in model.fields.iter() {
            content = content.push(system_choice_row(field));
        }
        content.spacing(16).into()
    }
}

fn page_radio(
    language: AppLanguage,
    label: UiText,
    value: SettingsPage,
    selected: SettingsPage,
) -> Element<'static, Message> {
    radio(
        ui_text(language, label),
        value,
        Some(selected),
        Message::SelectPage,
    )
    .into()
}

fn input_section_radio(
    language: AppLanguage,
    label: UiText,
    value: InputPageSection,
    selected: InputPageSection,
) -> Element<'static, Message> {
    input_section_radio_label(ui_text(language, label), value, selected)
}

fn input_section_radio_label(
    label: &'static str,
    value: InputPageSection,
    selected: InputPageSection,
) -> Element<'static, Message> {
    radio(label, value, Some(selected), Message::SelectInputSection).into()
}

fn labeled_pick_list<T: Clone + Eq + 'static>(
    label: &'static str,
    options: impl Into<Vec<Choice<T>>>,
    selected: Choice<T>,
    on_select: fn(Choice<T>) -> Message,
) -> Element<'static, Message> {
    let options = options.into();
    row![
        text(label).width(Length::Fixed(220.0)),
        pick_list(options, Some(selected), on_select).width(Length::Shrink)
    ]
    .spacing(12)
    .align_y(Alignment::Center)
    .into()
}

fn labeled_slider<'a>(
    label: &'static str,
    value: String,
    slider: impl Into<Element<'a, Message>>,
) -> Element<'a, Message> {
    row![
        text(label).width(Length::Fixed(220.0)),
        slider.into(),
        text(value).width(Length::Fixed(72.0)),
    ]
    .spacing(12)
    .align_y(Alignment::Center)
    .into()
}

fn selected_choice<T: Clone + Eq>(value: T, options: impl Into<Vec<Choice<T>>>) -> Choice<T> {
    options
        .into()
        .into_iter()
        .find(|choice| choice.value == value)
        .unwrap()
}

fn system_choice_row(field: &SystemSettingsFieldModel) -> Element<'static, Message> {
    let nerust_gui_shell::descriptor::SystemSettingsFieldKind::Choice { selected, options } =
        &field.kind;
    let choices = options
        .iter()
        .map(|option| Choice {
            value: option.id.as_str().to_string(),
            label: option.label.clone(),
        })
        .collect::<Vec<_>>();
    let selected = choices
        .iter()
        .find(|choice| choice.value == selected.as_str())
        .cloned()
        .or_else(|| choices.first().cloned())
        .unwrap_or(Choice {
            value: String::new(),
            label: String::new(),
        });
    let field_id = field.id.as_str().to_string();
    row![
        text(field.label.clone()).width(Length::Fixed(220.0)),
        pick_list(choices, Some(selected), move |choice| {
            Message::SetSystemChoice(field_id.clone(), choice)
        })
        .width(Length::Shrink)
    ]
    .spacing(12)
    .align_y(Alignment::Center)
    .into()
}

#[cfg(target_os = "windows")]
fn default_font() -> Font {
    Font::with_name("Yu Gothic UI")
}

#[cfg(not(target_os = "windows"))]
fn default_font() -> Font {
    Font::DEFAULT
}

fn language_options(language: AppLanguage) -> Vec<Choice<AppLanguage>> {
    vec![
        Choice {
            value: AppLanguage::SystemDefault,
            label: ui_text(language, UiText::SystemDefault).to_string(),
        },
        Choice {
            value: AppLanguage::Japanese,
            label: ui_text(language, UiText::Japanese).to_string(),
        },
        Choice {
            value: AppLanguage::English,
            label: ui_text(language, UiText::English).to_string(),
        },
    ]
}

fn storage_policy_options(language: AppLanguage) -> Vec<Choice<StoragePolicy>> {
    vec![
        Choice {
            value: StoragePolicy::Sidecar,
            label: ui_text(language, UiText::Sidecar).to_string(),
        },
        Choice {
            value: StoragePolicy::AppSharedData,
            label: ui_text(language, UiText::AppSharedData).to_string(),
        },
        Choice {
            value: StoragePolicy::CustomDirectory,
            label: ui_text(language, UiText::CustomDirectory).to_string(),
        },
    ]
}

fn scaling_options(language: AppLanguage) -> Vec<Choice<ScalingMode>> {
    vec![
        Choice {
            value: ScalingMode::FitToWindow,
            label: ui_text(language, UiText::FitToWindow).to_string(),
        },
        Choice {
            value: ScalingMode::X1,
            label: "1x".to_string(),
        },
        Choice {
            value: ScalingMode::X2,
            label: "2x".to_string(),
        },
        Choice {
            value: ScalingMode::X3,
            label: "3x".to_string(),
        },
        Choice {
            value: ScalingMode::X4,
            label: "4x".to_string(),
        },
        Choice {
            value: ScalingMode::X5,
            label: "5x".to_string(),
        },
    ]
}

fn sample_rate_options() -> Vec<Choice<u32>> {
    vec![
        Choice {
            value: 22_050,
            label: "22050".to_string(),
        },
        Choice {
            value: 44_100,
            label: "44100".to_string(),
        },
        Choice {
            value: 48_000,
            label: "48000".to_string(),
        },
    ]
}

fn input_topology() -> InputTopologyDescriptor {
    default_input_topology_descriptor()
}

fn keyboard_key_from_physical(physical: Physical) -> Option<KeyboardKey> {
    let Physical::Code(code) = physical else {
        return None;
    };
    Some(match code {
        Code::Digit0 => KeyboardKey::Digit0,
        Code::Digit1 => KeyboardKey::Digit1,
        Code::Digit2 => KeyboardKey::Digit2,
        Code::Digit3 => KeyboardKey::Digit3,
        Code::Digit4 => KeyboardKey::Digit4,
        Code::Digit5 => KeyboardKey::Digit5,
        Code::Digit6 => KeyboardKey::Digit6,
        Code::Digit7 => KeyboardKey::Digit7,
        Code::Digit8 => KeyboardKey::Digit8,
        Code::Digit9 => KeyboardKey::Digit9,
        Code::KeyA => KeyboardKey::KeyA,
        Code::KeyB => KeyboardKey::KeyB,
        Code::KeyC => KeyboardKey::KeyC,
        Code::KeyD => KeyboardKey::KeyD,
        Code::KeyE => KeyboardKey::KeyE,
        Code::KeyF => KeyboardKey::KeyF,
        Code::KeyG => KeyboardKey::KeyG,
        Code::KeyH => KeyboardKey::KeyH,
        Code::KeyI => KeyboardKey::KeyI,
        Code::KeyJ => KeyboardKey::KeyJ,
        Code::KeyK => KeyboardKey::KeyK,
        Code::KeyL => KeyboardKey::KeyL,
        Code::KeyM => KeyboardKey::KeyM,
        Code::KeyN => KeyboardKey::KeyN,
        Code::KeyO => KeyboardKey::KeyO,
        Code::KeyP => KeyboardKey::KeyP,
        Code::KeyQ => KeyboardKey::KeyQ,
        Code::KeyR => KeyboardKey::KeyR,
        Code::KeyS => KeyboardKey::KeyS,
        Code::KeyT => KeyboardKey::KeyT,
        Code::KeyU => KeyboardKey::KeyU,
        Code::KeyV => KeyboardKey::KeyV,
        Code::KeyW => KeyboardKey::KeyW,
        Code::KeyX => KeyboardKey::KeyX,
        Code::KeyY => KeyboardKey::KeyY,
        Code::KeyZ => KeyboardKey::KeyZ,
        Code::ArrowUp => KeyboardKey::ArrowUp,
        Code::ArrowDown => KeyboardKey::ArrowDown,
        Code::ArrowLeft => KeyboardKey::ArrowLeft,
        Code::ArrowRight => KeyboardKey::ArrowRight,
        Code::Enter | Code::NumpadEnter => KeyboardKey::Enter,
        Code::Escape => KeyboardKey::Escape,
        Code::Space => KeyboardKey::Space,
        Code::Tab => KeyboardKey::Tab,
        Code::F1 => KeyboardKey::F1,
        Code::F2 => KeyboardKey::F2,
        Code::F3 => KeyboardKey::F3,
        Code::F4 => KeyboardKey::F4,
        Code::F5 => KeyboardKey::F5,
        Code::F6 => KeyboardKey::F6,
        Code::F7 => KeyboardKey::F7,
        Code::F8 => KeyboardKey::F8,
        Code::F9 => KeyboardKey::F9,
        Code::F10 => KeyboardKey::F10,
        Code::F11 => KeyboardKey::F11,
        Code::F12 => KeyboardKey::F12,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::{Message, close_requested_event, keyboard_key_from_physical};
    use iced::Event;
    use iced::event::Status;
    use iced::keyboard::key::{Code, Physical};
    use iced::window;
    use nerust_gui_settings::input::KeyboardKey;

    #[test]
    fn physical_key_mapping_matches_tao_bindings() {
        assert_eq!(
            keyboard_key_from_physical(Physical::Code(Code::KeyZ)),
            Some(KeyboardKey::KeyZ)
        );
        assert_eq!(
            keyboard_key_from_physical(Physical::Code(Code::ArrowLeft)),
            Some(KeyboardKey::ArrowLeft)
        );
        assert_eq!(
            keyboard_key_from_physical(Physical::Code(Code::F11)),
            Some(KeyboardKey::F11)
        );
        assert_eq!(
            keyboard_key_from_physical(Physical::Code(Code::Delete)),
            None
        );
    }

    #[test]
    fn close_requested_event_maps_to_save_message() {
        assert!(matches!(
            close_requested_event(
                Event::Window(window::Event::CloseRequested),
                Status::Ignored,
                window::Id::unique(),
            ),
            Some(Message::CloseRequested)
        ));
    }
}
