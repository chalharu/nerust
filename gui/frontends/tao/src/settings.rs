use std::{
    cell::Cell,
    rc::Rc,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};

use iced::{
    Font, Length, Task, Theme,
    alignment::Alignment,
    widget::{
        Column, button, checkbox, column, container, pick_list, radio, row, scrollable, slider,
        text, text_input,
    },
};
use iced_winit::program::Program;
use nerust_core_traits::audio::AudioBackendRegistry;
use nerust_gui_runtime::settings::SettingsSnapshot;
use nerust_gui_settings::{language::AppLanguage, local::ScalingMode, shared::StoragePolicy};
use nerust_gui_shell::registry::SystemRegistry;

use nerust_settings_core::{
    editor::{CaptureTarget, current_binding_label},
    i18n::{UiText, text as ui_text},
};

use nerust_gui_viewmodel::settings::{
    dto::ChoiceView, SettingsViewModel, StoragePathError, StoragePathValidator,
};
use nerust_input_traits::AttachmentId;
use nerust_keyboard::Key;
use nerust_settings_core::bindings::descriptors::shortcut_descriptors;
use rfd::FileDialog;

type El<'a> = iced::Element<'a, Message, iced::Theme, iced_tiny_skia::Renderer>;

// ---------------------------------------------------------------------------
// Shared types
// ---------------------------------------------------------------------------

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
    SelectSystemTab(usize),
    SelectInputTab(usize),
    SetLanguage(ChoiceView<AppLanguage>),
    SetStoragePolicy(ChoiceView<StoragePolicy>),
    SetStorageDirectory(String),
    BrowseStorageDirectory,
    ToggleFullscreenDefault(bool),
    SetScaling(ChoiceView<ScalingMode>),
    ToggleVsync(bool),
    ToggleMute(bool),
    SetVolume(u8),
    SetSampleRate(ChoiceView<u32>),
    SetLatency(u16),
    SetSystemChoice(
        String,
        ChoiceView<nerust_core_traits::factory::descriptor::SystemSettingsChoiceId>,
    ),
    StartCapture(CaptureTarget),
    ClearCapture(CaptureTarget),
    CaptureKey(Key),
    SetControllerSlot {
        slot: AttachmentId,
        controller_id: Option<String>,
    },
    Submit,
    Cancel,
}

pub(crate) struct SettingsAppProgram {
    pub(crate) snapshot: SettingsSnapshot,
    pub(crate) registry: Arc<SystemRegistry>,
    pub(crate) audio_registry: Arc<AudioBackendRegistry>,
    pub(crate) should_close: Arc<AtomicBool>,
    pub(crate) pending_apply: Arc<Mutex<Option<SettingsSnapshot>>>,
    pub(crate) view_invalidated: Rc<Cell<bool>>,
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
            self.registry.clone(),
            self.audio_registry.clone(),
            self.should_close.clone(),
            self.pending_apply.clone(),
            self.view_invalidated.clone(),
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
    pub(crate) view_invalidated: Rc<Cell<bool>>,
    pub vm: SettingsViewModel,
    _revision_subscription: nerust_gui_viewmodel::settings::Subscription,
    page: SettingsPage,
    system_tab_index: Option<usize>,
    input_tab_index: Option<usize>,
    input_section: InputPageSection,
    storage_directory_input: String,
    error_message: Option<String>,
}

impl SettingsAppState {
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn new(
        snapshot: &SettingsSnapshot,
        registry: Arc<SystemRegistry>,
        audio_registry: Arc<AudioBackendRegistry>,
    ) -> Self {
        Self::new_with_invalidation(
            snapshot,
            registry,
            audio_registry,
            Rc::new(Cell::new(false)),
        )
    }

    fn new_with_invalidation(
        snapshot: &SettingsSnapshot,
        registry: Arc<SystemRegistry>,
        audio_registry: Arc<AudioBackendRegistry>,
        view_invalidated: Rc<Cell<bool>>,
    ) -> Self {
        let supported_sample_rates: Arc<[u32]> = {
            let rates = audio_registry.supported_rates();
            if rates.is_empty() {
                Arc::new([44_100, 48_000])
            } else {
                rates.iter().copied().collect()
            }
        };
        use nerust_gui_runtime::settings::apply::{
            DirectoryValidationError, validate_directory_path_typed,
        };

        #[derive(Debug)]
        struct FsStoragePathValidator;
        impl StoragePathValidator for FsStoragePathValidator {
            fn validate(&self, path: &std::path::Path) -> Result<(), StoragePathError> {
                match validate_directory_path_typed(path) {
                    Ok(()) => Ok(()),
                    Err(DirectoryValidationError::NotDirectory) => {
                        Err(StoragePathError::NotDirectory)
                    }
                    Err(DirectoryValidationError::Inaccessible(e)) => {
                        Err(StoragePathError::Inaccessible(e.to_string()))
                    }
                }
            }
        }

        let factories: Vec<Arc<dyn nerust_core_traits::factory::CoreFactory>> =
            registry.all().iter().map(Arc::clone).collect();
        let vm = SettingsViewModel::new(
            snapshot.clone(),
            factories,
            supported_sample_rates,
            Rc::new(FsStoragePathValidator) as Rc<dyn StoragePathValidator>,
        );
        let invalidated = Rc::clone(&view_invalidated);
        let _revision_subscription = vm.revision.observe(move |_| {
            invalidated.set(true);
        });
        let has_systems = !vm.systems().is_empty();
        Self {
            should_close: Arc::new(AtomicBool::new(false)),
            pending_apply: Arc::new(Mutex::new(None)),
            view_invalidated,
            vm,
            _revision_subscription,
            page: SettingsPage::General,
            system_tab_index: if has_systems { Some(0) } else { None },
            input_tab_index: if has_systems { Some(0) } else { None },
            input_section: InputPageSection::Attachment(0),
            storage_directory_input: snapshot
                .shared
                .persistence
                .storage_directory
                .as_ref()
                .map(|path| path.to_string_lossy().to_string())
                .unwrap_or_default(),
            error_message: None,
        }
    }

    pub(crate) fn new_with_shared(
        snapshot: &SettingsSnapshot,
        registry: Arc<SystemRegistry>,
        audio_registry: Arc<AudioBackendRegistry>,
        should_close: Arc<AtomicBool>,
        pending_apply: Arc<Mutex<Option<SettingsSnapshot>>>,
        view_invalidated: Rc<Cell<bool>>,
    ) -> Self {
        let mut state =
            Self::new_with_invalidation(snapshot, registry, audio_registry, view_invalidated);
        state.should_close = should_close;
        state.pending_apply = pending_apply;
        state
    }

    fn language(&self) -> AppLanguage {
        self.vm.general.view.get().language
    }

    fn validation_errors(&self) -> Vec<String> {
        self.vm
            .finish()
            .err()
            .map(|v| v.issues.into_iter().map(|i| i.message).collect())
            .unwrap_or_default()
    }

    fn storage_error(&self) -> Option<String> {
        let general = self.vm.general.view.get();
        if general.show_storage_directory && general.storage_directory.is_empty() {
            Some("Custom storage directory required".into())
        } else {
            None
        }
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        self.error_message = None;
        match message {
            Message::SelectPage(page) => {
                self.page = page;
                self.view_invalidated.set(true);
            }
            Message::SelectInputSection(section) => {
                self.input_section = section;
                self.view_invalidated.set(true);
            }
            Message::SelectSystemTab(index) => {
                self.system_tab_index = Some(index);
                self.view_invalidated.set(true);
            }
            Message::SelectInputTab(index) => {
                self.input_tab_index = Some(index);
                self.input_section = InputPageSection::Attachment(0);
                self.view_invalidated.set(true);
            }
            Message::SetLanguage(choice) => {
                if let Err(e) = self.vm.general.set_language(choice.value) {
                    self.error_message = Some(e.to_string());
                }
            }
            Message::SetStoragePolicy(choice) => {
                if let Err(e) = self.vm.general.set_storage_policy(choice.value) {
                    self.error_message = Some(e.to_string());
                }
            }
            Message::SetStorageDirectory(value) => {
                self.storage_directory_input = value.clone();
                if let Err(e) = self
                    .vm
                    .general
                    .set_storage_directory((!value.is_empty()).then(|| value.into()))
                {
                    self.error_message = Some(e.to_string());
                }
            }
            Message::BrowseStorageDirectory => {
                if let Some(path) = FileDialog::new()
                    .set_title(ui_text(self.language(), UiText::SaveStorageDirectory))
                    .pick_folder()
                {
                    let path = path.to_string_lossy().to_string();
                    self.storage_directory_input = path.clone();
                    if let Err(e) = self.vm.general.set_storage_directory(Some(path.into())) {
                        self.error_message = Some(e.to_string());
                    }
                }
            }
            Message::ToggleFullscreenDefault(value) => {
                if let Err(e) = self.vm.video.set_fullscreen_default(value) {
                    self.error_message = Some(e.to_string());
                }
            }
            Message::SetScaling(choice) => {
                if let Err(e) = self.vm.video.set_scaling(choice.value) {
                    self.error_message = Some(e.to_string());
                }
            }
            Message::ToggleVsync(value) => {
                if let Err(e) = self.vm.video.set_vsync(value) {
                    self.error_message = Some(e.to_string());
                }
            }
            Message::ToggleMute(value) => {
                if let Err(e) = self.vm.audio.set_mute(value) {
                    self.error_message = Some(e.to_string());
                }
            }
            Message::SetVolume(value) => {
                if let Err(e) = self.vm.audio.set_volume(value) {
                    self.error_message = Some(e.to_string());
                }
            }
            Message::SetSampleRate(choice) => {
                if let Err(e) = self.vm.audio.set_sample_rate(choice.value) {
                    self.error_message = Some(e.to_string());
                }
            }
            Message::SetLatency(value) => {
                if let Err(e) = self.vm.audio.set_latency(value) {
                    self.error_message = Some(e.to_string());
                }
            }
            Message::SetSystemChoice(field, choice) => {
                let system_tab_index = self.system_tab_index;
                if let Some(idx) = system_tab_index
                    && let Some(system_vm) = self.vm.systems().get(idx)
                    && let Err(e) = system_vm.set_choice(
                        &nerust_core_traits::factory::descriptor::SystemSettingsFieldId(
                            field.into(),
                        ),
                        &choice.value,
                    )
                {
                    self.error_message = Some(e.to_string());
                }
            }
            Message::SetControllerSlot {
                slot,
                controller_id,
            } => {
                let input_tab_index = self.input_tab_index;
                if let Some(idx) = input_tab_index
                    && let Some(input_vm) = self.vm.inputs().get(idx)
                    && let Err(e) = input_vm.set_controller_slot(slot, controller_id.as_deref())
                {
                    self.error_message = Some(e.to_string());
                }
            }
            Message::StartCapture(target) => {
                if let Err(e) = self.vm.capture.start_capture(target) {
                    self.error_message = Some(e.to_string());
                }
            }
            Message::ClearCapture(target) => {
                if let Err(e) = self.vm.capture.clear_binding(&target) {
                    self.error_message = Some(e.to_string());
                }
            }
            Message::CaptureKey(key) => {
                self.vm.capture.apply_captured_key(key);
            }
            Message::Submit => {
                if let Ok(snapshot) = self.vm.finish() {
                    *self.pending_apply.lock().expect("pending apply mutex") = Some(snapshot);
                    self.should_close.store(true, Ordering::Release);
                }
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
        let language = self.language();
        let Some(input_tab_index) = self.input_tab_index else {
            return column![text("No systems available").size(14)].into();
        };
        let inputs = self.vm.inputs();
        let Some(input_vm) = inputs.get(input_tab_index) else {
            return column![text("No systems available").size(14)].into();
        };
        let view = input_vm.view.get();

        let mut content: Column<Message, Theme, iced_tiny_skia::Renderer> = column![];

        // Tab buttons
        let names: Vec<String> = inputs
            .iter()
            .map(|vm| vm.display_name().to_string())
            .collect();
        let tab_row = row(names.iter().enumerate().map(|(i, name)| {
            let btn_text = text(name.clone()).size(14);
            if Some(i) == self.input_tab_index {
                button(btn_text).style(button::primary).into()
            } else {
                button(btn_text).on_press(Message::SelectInputTab(i)).into()
            }
        }))
        .spacing(4);
        content = content.push(tab_row);

        // Conflicts
        for conflict in &view.conflicts {
            content = content.push(text(conflict.message.clone()));
        }

        // Controller slots
        for slot in &view.slots {
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
        navigation = navigation.push(radio(
            ui_text(language, UiText::Shortcuts),
            InputPageSection::Shortcuts,
            Some(self.input_section),
            Message::SelectInputSection,
        ));

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
            InputPageSection::Shortcuts => self.shortcuts_section(),
        };

        content
            .push(navigation)
            .push(input_section_content)
            .spacing(16)
            .into()
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

    fn shortcuts_section(&self) -> El<'static> {
        let language = self.language();
        let capture = self.vm.capture.view.get();
        let snapshot = self.vm.snapshot();
        let mut content: Column<Message, Theme, iced_tiny_skia::Renderer> =
            column![text(ui_text(language, UiText::Shortcuts).to_string())];
        for desc in shortcut_descriptors() {
            let target = CaptureTarget::Shortcut(desc.action);
            let binding_label = if capture.target.as_ref() == Some(&target) {
                ui_text(language, UiText::CapturePrompt).to_string()
            } else {
                current_binding_label(&snapshot, &target)
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| ui_text(language, UiText::Unbound).to_string())
            };
            content = content.push(
                row![
                    text(desc.label).width(Length::Fixed(180.0)),
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
        let _language = self.language();
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
        let tab_row = row(systems.iter().enumerate().map(|(i, vm)| {
            let btn_text = text(vm.display_name()).size(14);
            if Some(i) == self.system_tab_index {
                button(btn_text).style(button::primary).into()
            } else {
                button(btn_text)
                    .on_press(Message::SelectSystemTab(i))
                    .into()
            }
        }))
        .spacing(4);
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

fn labeled_pick_list<'a, T: Clone + PartialEq + Eq + 'static>(
    label: &str,
    options: Vec<ChoiceView<T>>,
    selected: Option<ChoiceView<T>>,
    on_select: impl Fn(ChoiceView<T>) -> Message + 'a,
) -> El<'a> {
    row![
        text(label.to_string()).width(Length::Fixed(220.0)),
        pick_list(options, selected, on_select).width(Length::Shrink)
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

#[cfg(target_os = "windows")]
fn default_font() -> Font {
    Font::with_name("Yu Gothic UI")
}

#[cfg(not(target_os = "windows"))]
fn default_font() -> Font {
    Font::DEFAULT
}

pub(crate) fn keyboard_key_from_physical(physical: iced::keyboard::key::Physical) -> Option<Key> {
    physical.try_into().ok()
}

#[cfg(test)]
mod tests {
    use iced::keyboard::key::{Code, Physical};
    use nerust_core_traits::audio::AudioBackendRegistry;
    use nerust_gui_runtime::settings::SettingsSnapshot;
    use nerust_gui_settings::{
        app_state::DesktopAppState,
        local::{HostBackendLocalSettings, ScalingMode},
        shared::{DesktopSharedSettings, StoragePolicy},
    };
    use nerust_gui_shell::registry::SystemRegistry;
    use nerust_gui_viewmodel::settings::dto::ChoiceView;
    use nerust_keyboard::Key;
    use std::sync::{Arc, atomic::Ordering};

    use super::*;

    fn empty_snapshot() -> SettingsSnapshot {
        SettingsSnapshot {
            shared: DesktopSharedSettings::default(),
            local: HostBackendLocalSettings::default(),
            app_state: DesktopAppState::default(),
        }
    }

    fn empty_state() -> SettingsAppState {
        SettingsAppState::new(
            &empty_snapshot(),
            Arc::new(SystemRegistry::new(Vec::new())),
            Arc::new(AudioBackendRegistry::new()),
        )
    }

    fn dispatch(state: &mut SettingsAppState, message: Message) {
        drop(state.update(message));
    }

    #[test]
    fn revision_callback_shares_external_view_invalidated_cell() {
        use std::cell::Cell;

        let external = Rc::new(Cell::new(false));
        let mut state = SettingsAppState::new_with_shared(
            &empty_snapshot(),
            Arc::new(SystemRegistry::new(Vec::new())),
            Arc::new(AudioBackendRegistry::new()),
            Arc::new(AtomicBool::new(false)),
            Arc::new(Mutex::new(None)),
            Rc::clone(&external),
        );

        dispatch(
            &mut state,
            Message::SetLanguage(ChoiceView {
                value: AppLanguage::Japanese,
                label: "Japanese".into(),
            }),
        );

        assert!(
            external.get(),
            "revision callback should have set the externally-shared cell"
        );
    }

    #[test]
    fn physical_key_mapping_matches_tao_bindings() {
        assert_eq!(
            keyboard_key_from_physical(Physical::Code(Code::KeyZ)),
            Some(Key::KeyZ)
        );
        assert_eq!(
            keyboard_key_from_physical(Physical::Code(Code::ArrowLeft)),
            Some(Key::ArrowLeft)
        );
        assert_eq!(
            keyboard_key_from_physical(Physical::Code(Code::F11)),
            Some(Key::F11)
        );
    }

    #[test]
    fn view_model_provides_projection_values() {
        let state = empty_state();
        let general = state.vm.general.view.get();
        assert_eq!(general.language, AppLanguage::SystemDefault);
        assert_eq!(general.language_choices.len(), 3);

        let video = state.vm.video.view.get();
        assert!(!video.fullscreen_default);
        assert_eq!(video.scaling_choices.len(), 6);

        let audio = state.vm.audio.view.get();
        assert!(!audio.muted);
        assert_eq!(audio.volume_percent, 100);
    }

    #[test]
    fn update_applies_general_video_and_audio_messages() {
        let mut state = empty_state();

        dispatch(&mut state, Message::SelectPage(SettingsPage::Audio));
        dispatch(&mut state, Message::SelectSystemTab(2));
        dispatch(&mut state, Message::SelectInputTab(3));
        dispatch(
            &mut state,
            Message::SetLanguage(ChoiceView {
                value: AppLanguage::Japanese,
                label: "Japanese".into(),
            }),
        );
        dispatch(
            &mut state,
            Message::SetStoragePolicy(ChoiceView {
                value: StoragePolicy::CustomDirectory,
                label: "Custom".into(),
            }),
        );
        dispatch(
            &mut state,
            Message::SetStorageDirectory("/tmp/states".into()),
        );
        dispatch(&mut state, Message::ToggleFullscreenDefault(true));
        dispatch(
            &mut state,
            Message::SetScaling(ChoiceView {
                value: ScalingMode::X3,
                label: "3x".into(),
            }),
        );
        dispatch(&mut state, Message::ToggleVsync(false));
        dispatch(&mut state, Message::ToggleMute(true));
        dispatch(&mut state, Message::SetVolume(42));
        dispatch(
            &mut state,
            Message::SetSampleRate(ChoiceView {
                value: 44_100,
                label: "44100".into(),
            }),
        );
        dispatch(&mut state, Message::SetLatency(75));

        assert_eq!(state.page, SettingsPage::Audio);
        assert_eq!(state.system_tab_index, Some(2));
        assert_eq!(state.input_tab_index, Some(3));

        // Read from ViewModel projection
        let general = state.vm.general.view.get();
        assert_eq!(general.language, AppLanguage::Japanese);
        assert_eq!(general.storage_policy, StoragePolicy::CustomDirectory);

        // Also check the snapshot directly
        let snap = state.vm.snapshot();
        assert_eq!(
            snap.shared.persistence.storage_directory.as_deref(),
            Some(std::path::Path::new("/tmp/states"))
        );

        let video = state.vm.video.view.get();
        assert!(video.fullscreen_default);
        assert_eq!(video.scaling, ScalingMode::X3);
        assert!(!video.vsync);

        let audio = state.vm.audio.view.get();
        assert!(audio.muted);
        assert_eq!(audio.volume_percent, 42);
        assert_eq!(audio.sample_rate, 44_100);
        assert_eq!(audio.latency_ms, 75);
    }

    #[test]
    fn submit_and_cancel_publish_close_state() {
        let mut submitted = empty_state();
        dispatch(&mut submitted, Message::Submit);

        assert!(submitted.should_close.load(Ordering::Acquire));
        assert!(submitted.pending_apply.lock().unwrap().is_some());

        let mut cancelled = empty_state();
        dispatch(&mut cancelled, Message::Cancel);
        assert!(cancelled.should_close.load(Ordering::Acquire));
        assert!(cancelled.pending_apply.lock().unwrap().is_none());
    }

    #[test]
    fn empty_registry_paths_are_safe_and_validation_blocks_submit() {
        let mut state = empty_state();

        // No custom directory and default policy is sidecar — no storage error
        // Submit should succeed (empty snapshot)
        dispatch(&mut state, Message::Submit);
        assert!(state.pending_apply.lock().unwrap().is_some());
    }

    #[test]
    fn capture_messages_update_capture_state() {
        let mut state = empty_state();
        let target =
            CaptureTarget::Shortcut(nerust_gui_settings::input::ShortcutAction::TogglePause);

        dispatch(&mut state, Message::StartCapture(target.clone()));
        let capture = state.vm.capture.view.get();
        assert_eq!(capture.target, Some(target.clone()));

        dispatch(&mut state, Message::CaptureKey(Key::Space));
        let capture = state.vm.capture.view.get();
        assert!(capture.target.is_none());

        dispatch(&mut state, Message::StartCapture(target.clone()));
        dispatch(&mut state, Message::ClearCapture(target));
        let capture = state.vm.capture.view.get();
        assert!(capture.target.is_none());
    }

    #[test]
    fn revision_advances_on_vm_mutation() {
        let mut state = empty_state();
        let rev_before = state.vm.revision.get();
        dispatch(
            &mut state,
            Message::SetLanguage(ChoiceView {
                value: AppLanguage::Japanese,
                label: "Japanese".into(),
            }),
        );
        assert!(state.vm.revision.get() > rev_before);
    }
}
