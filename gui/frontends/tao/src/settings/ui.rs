use iced::alignment::Alignment;
use iced::keyboard::key::Code;
use iced::widget::{
    button, checkbox, column, container, pick_list, radio, row, scrollable, slider, text,
    text_input,
};
use iced::{Font, Length, Task, Theme};
use iced_winit::program::Program;
use nerust_factory_nes::NesFactory;
use nerust_gui_runtime::settings::SettingsSnapshot;
use nerust_gui_runtime::settings::apply::validate_shared_settings;
use nerust_gui_settings::input::KeyboardKey;
use nerust_gui_settings::language::AppLanguage;
use nerust_gui_settings::local::ScalingMode;
use nerust_gui_settings::shared::StoragePolicy;
use nerust_gui_shell::descriptor::{
    SystemSettingsChoiceId, SystemSettingsFieldKind, SystemSettingsFieldModel,
};
use nerust_gui_shell::factory::CoreFactory;
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
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

type El<'a> = iced::Element<'a, Message, iced::Theme, iced_tiny_skia::Renderer>;

// ---------------------------------------------------------------------------
// Shared types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Choice<T: Clone + Eq> {
    value: T,
    label: String,
}

impl<T: Clone + Eq> fmt::Display for Choice<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.label)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SettingsPage {
    General,
    Input,
    Video,
    Audio,
    System,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum InputPageSection {
    Attachment(usize),
    Shortcuts,
}

#[derive(Debug, Clone)]
pub(crate) enum Message {
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
    Submit,
    Cancel,
}

// ---------------------------------------------------------------------------
// Old path (dead code in PR2, removed in PR3 -- child process iced::application)
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// New path: Program + State (iced_winit integration)
// ---------------------------------------------------------------------------

pub(crate) struct SettingsAppProgram {
    pub(crate) snapshot: SettingsSnapshot,
    pub(crate) should_close: Arc<AtomicBool>,
    pub(crate) pending_apply: Arc<Mutex<Option<SettingsSnapshot>>>,
    pub(crate) capture_target: Arc<Mutex<Option<CaptureTarget>>>,
}

impl Program for SettingsAppProgram {
    type State = SettingsAppState;
    type Message = Message;
    type Theme = Theme;
    type Renderer = iced_tiny_skia::Renderer;
    type Executor = iced_winit::futures::backend::default::Executor;

    fn name() -> &'static str {
        "nerust_settings"
    }

    fn settings(&self) -> iced::Settings {
        iced::Settings {
            default_font: default_font(),
            default_text_size: iced::Pixels(16.0),
            ..Default::default()
        }
    }

    fn boot(&self) -> (Self::State, Task<Self::Message>) {
        let state = SettingsAppState::new_with_shared(
            &self.snapshot,
            self.should_close.clone(),
            self.pending_apply.clone(),
            self.capture_target.clone(),
        );
        (state, Task::none())
    }

    fn update(&self, state: &mut Self::State, message: Self::Message) -> Task<Self::Message> {
        state.update(message)
    }

    fn view<'a>(
        &self,
        state: &'a Self::State,
        _window: iced::window::Id,
    ) -> iced::Element<'a, Self::Message, Self::Theme, Self::Renderer> {
        state.view()
    }

    fn window(&self) -> Option<iced::window::Settings> {
        None
    }
}

// ---------------------------------------------------------------------------
// SettingsAppState
// ---------------------------------------------------------------------------

pub(crate) struct SettingsAppState {
    pub(crate) should_close: Arc<AtomicBool>,
    pub(crate) pending_apply: Arc<Mutex<Option<SettingsSnapshot>>>,
    pub(crate) capture_target: Arc<Mutex<Option<CaptureTarget>>>,
    factory: Arc<dyn CoreFactory>,
    draft: SettingsSnapshot,
    page: SettingsPage,
    input_section: InputPageSection,
    storage_directory_input: String,
    error_message: Option<String>,
}

impl SettingsAppState {
    pub(crate) fn new(snapshot: &SettingsSnapshot) -> Self {
        let storage_directory_input = snapshot
            .shared
            .persistence
            .storage_directory
            .as_ref()
            .map(|path| path.to_string_lossy().to_string())
            .unwrap_or_default();
        Self {
            should_close: Arc::new(AtomicBool::new(false)),
            pending_apply: Arc::new(Mutex::new(None)),
            capture_target: Arc::new(Mutex::new(None)),
            factory: Arc::new(NesFactory),
            draft: snapshot.clone(),
            page: SettingsPage::General,
            input_section: InputPageSection::Attachment(0),
            storage_directory_input,
            error_message: None,
        }
    }

    pub(crate) fn new_with_shared(
        snapshot: &SettingsSnapshot,
        should_close: Arc<AtomicBool>,
        pending_apply: Arc<Mutex<Option<SettingsSnapshot>>>,
        capture_target: Arc<Mutex<Option<CaptureTarget>>>,
    ) -> Self {
        let mut state = Self::new(snapshot);
        state.should_close = should_close;
        state.pending_apply = pending_apply;
        state.capture_target = capture_target;
        state
    }

    fn language(&self) -> AppLanguage {
        self.draft.shared.general.language
    }

    fn validation_errors(&self) -> Vec<String> {
        let mut errors = Vec::new();
        if let Err(error) = validate_shared_settings(&self.draft.shared) {
            errors.push(error.to_string());
        }
        for (key, labels) in conflicting_keys(&self.draft.shared, &input_topology(self)) {
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
        let (key, labels) = conflicting_keys(&self.draft.shared, &input_topology(self))
            .into_iter()
            .next()?;
        Some(format!(
            "{}: {}",
            keyboard_key_label(key),
            labels.join(", ")
        ))
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        self.error_message = None;
        match message {
            Message::SelectPage(page) => self.page = page,
            Message::SelectInputSection(section) => self.input_section = section,
            Message::SetLanguage(choice) => self.draft.shared.general.language = choice.value,
            Message::SetStoragePolicy(choice) => {
                self.draft.shared.persistence.storage_policy = choice.value;
            }
            Message::SetStorageDirectory(value) => {
                self.storage_directory_input = value;
                self.draft.shared.persistence.storage_directory =
                    (!self.storage_directory_input.is_empty())
                        .then(|| self.storage_directory_input.clone().into());
            }
            Message::BrowseStorageDirectory => {
                if let Some(path) = FileDialog::new()
                    .set_title(ui_text(self.language(), UiText::SaveStorageDirectory))
                    .pick_folder()
                {
                    let path = path.to_string_lossy().to_string();
                    self.storage_directory_input = path.clone();
                    self.draft.shared.persistence.storage_directory = Some(path.into());
                }
            }
            Message::ToggleFullscreenDefault(value) => {
                self.draft.local.video.window.fullscreen_default = value;
            }
            Message::SetScaling(choice) => self.draft.local.video.window.scaling = choice.value,
            Message::ToggleVsync(value) => self.draft.local.video.presentation.vsync = value,
            Message::ToggleMute(value) => self.draft.local.audio.muted = value,
            Message::SetVolume(value) => self.draft.local.audio.master_volume_percent = value,
            Message::SetSampleRate(choice) => self.draft.local.audio.sample_rate = choice.value,
            Message::SetLatency(value) => self.draft.local.audio.latency_ms = value,
            Message::SetSystemChoice(field, choice) => {
                let _ = self.factory.apply_settings_choice(
                    &mut self.draft,
                    &nerust_gui_shell::descriptor::SystemSettingsFieldId(field.into()),
                    &SystemSettingsChoiceId(choice.value.into()),
                );
            }
            Message::StartCapture(target) => {
                *self.capture_target.lock().unwrap() = Some(target);
            }
            Message::ClearCapture(target) => {
                apply_capture_target(&mut self.draft, &target, None);
                self.capture_target.lock().unwrap().take();
            }
            Message::CaptureKey(key) => {
                let target = self.capture_target.lock().unwrap().take();
                if let Some(target) = target {
                    apply_capture_target(&mut self.draft, &target, Some(key));
                }
            }
            Message::Submit => {
                if !self.validation_errors().is_empty() {
                    return Task::none();
                }
                *self.pending_apply.lock().unwrap() = Some(self.draft.clone());
                self.should_close.store(true, Ordering::Release);
            }
            Message::Cancel => {
                self.should_close.store(true, Ordering::Release);
            }
        }
        Task::none()
    }

    fn view(&self) -> El<'_> {
        let language = self.language();
        let validation_errors = self.validation_errors();
        let can_submit = validation_errors.is_empty();

        let sidebar = column![
            page_radio(language, UiText::General, SettingsPage::General, self.page),
            page_radio(language, UiText::Input, SettingsPage::Input, self.page),
            page_radio(language, UiText::Video, SettingsPage::Video, self.page),
            page_radio(language, UiText::Audio, SettingsPage::Audio, self.page),
            page_radio(language, UiText::System, SettingsPage::System, self.page),
        ]
        .spacing(10)
        .width(Length::Shrink);

        let content = scrollable(
            container(self.page_content())
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

        if let Some(error_message) = self.error_message.as_ref() {
            root = root.push(text(error_message.clone()));
        } else if let Some(first_error) = validation_errors.first() {
            root = root.push(text(first_error.clone()));
        }

        let buttons = row![
            button(ui_text(language, UiText::Cancel)).on_press(Message::Cancel),
            button(ui_text(language, UiText::Ok))
                .on_press_maybe(can_submit.then_some(Message::Submit)),
        ]
        .spacing(12)
        .align_y(Alignment::Center);

        root.push(container(buttons).width(Length::Fill)).into()
    }

    fn page_content(&self) -> El<'_> {
        match self.page {
            SettingsPage::General => self.general_page(),
            SettingsPage::Input => self.input_page(),
            SettingsPage::Video => self.video_page(),
            SettingsPage::Audio => self.audio_page(),
            SettingsPage::System => self.system_page(),
        }
    }

    fn general_page(&self) -> El<'_> {
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

    fn input_page(&self) -> El<'_> {
        let language = self.language();
        let mut content = column![];
        if let Some(conflict) = self.input_conflict() {
            content = content.push(text(conflict));
        }

        let sections = keyboard_binding_sections(&input_topology(self));
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
    ) -> El<'a> {
        let language = self.language();
        let current_capture = self.capture_target.lock().unwrap();
        let mut content = column![text(title)];
        for (label, target) in rows {
            let binding_label = if current_capture.as_ref() == Some(&target) {
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

    fn video_page(&self) -> El<'_> {
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

    fn audio_page(&self) -> El<'_> {
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

    fn system_page(&self) -> El<'_> {
        let model = self.factory.settings_page(&self.draft);
        let mut content = column![];
        for field in model.fields.iter() {
            content = content.push(system_choice_row(field));
        }
        content.spacing(16).into()
    }
}

// ---------------------------------------------------------------------------
// Helper functions (shared between old and new paths)
// ---------------------------------------------------------------------------

fn page_radio(
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

fn input_section_radio(
    language: AppLanguage,
    label: UiText,
    value: InputPageSection,
    selected: InputPageSection,
) -> El<'static> {
    input_section_radio_label(ui_text(language, label), value, selected)
}

fn input_section_radio_label(
    label: &'static str,
    value: InputPageSection,
    selected: InputPageSection,
) -> El<'static> {
    radio(label, value, Some(selected), Message::SelectInputSection).into()
}

fn labeled_pick_list<T: Clone + Eq + 'static>(
    label: &'static str,
    options: impl Into<Vec<Choice<T>>>,
    selected: Choice<T>,
    on_select: fn(Choice<T>) -> Message,
) -> El<'static> {
    let options = options.into();
    row![
        text(label).width(Length::Fixed(220.0)),
        pick_list(options, Some(selected), on_select).width(Length::Shrink)
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

fn selected_choice<T: Clone + Eq>(value: T, options: impl Into<Vec<Choice<T>>>) -> Choice<T> {
    options
        .into()
        .into_iter()
        .find(|choice| choice.value == value)
        .unwrap()
}

fn system_choice_row(field: &SystemSettingsFieldModel) -> El<'static> {
    let SystemSettingsFieldKind::Choice { selected, options } = &field.kind;
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

const FALLBACK_SAMPLE_RATES: [u32; 2] = [44_100, 48_000];

fn sample_rate_options() -> Vec<Choice<u32>> {
    let rates = nerust_gui_shell::settings::audio_registry().supported_rates();
    let rates = if rates.is_empty() {
        &FALLBACK_SAMPLE_RATES
    } else {
        rates
    };
    rates
        .iter()
        .map(|&r| Choice {
            value: r,
            label: format!("{r}"),
        })
        .collect()
}

fn input_topology(state: &SettingsAppState) -> InputTopologyDescriptor {
    state.factory.system_descriptor().input_topology
}

pub(crate) fn keyboard_key_from_physical(
    physical: iced::keyboard::key::Physical,
) -> Option<KeyboardKey> {
    let iced::keyboard::key::Physical::Code(code) = physical else {
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
    use super::keyboard_key_from_physical;

    use iced::keyboard::key::{Code, Physical};

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
}
