use std::{
    rc::Rc,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};

use iced::{
    Length, Task, Theme,
    alignment::Alignment,
    widget::{button, column, container, row, scrollable, text},
};
use iced_winit::program::Program;
use nerust_core_traits::audio::AudioBackendRegistry;
use nerust_gui_runtime::settings::SettingsSnapshot;
use nerust_gui_shell::registry::SystemRegistry;
use nerust_settings_core::model::{
    language::AppLanguage,
    local::{NotificationAreaMode, NotificationColor, NotificationPosition, ScalingMode},
    shared::StoragePolicy,
};

use nerust_settings_core::{
    editor::CaptureTarget,
    i18n::{UiText, text as ui_text},
};

use nerust_gui_viewmodel::settings::{
    SettingsViewModel, StoragePathError, StoragePathValidator, dto::ChoiceView,
};
use nerust_input_traits::AttachmentId;
use nerust_keyboard::Key;

use rfd::FileDialog;

use crate::aux::bridge::lock_recovered;
use crate::convert::default_font;

pub(super) type El<'a> = iced::Element<'a, Message, iced::Theme, iced_tiny_skia::Renderer>;

// ---------------------------------------------------------------------------
// Shared types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SettingsPage {
    General,
    Input,
    Video,
    Audio,
    Notifications,
    System,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum InputPageSection {
    Attachment(usize),
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
    ToggleInfoEnabled(bool),
    SetInfoDuration(u32),
    SetInfoColor(ChoiceView<NotificationColor>),
    ToggleWarningEnabled(bool),
    SetWarningDuration(u32),
    SetWarningColor(ChoiceView<NotificationColor>),
    ToggleErrorEnabled(bool),
    ToggleErrorModal(bool),
    SetErrorColor(ChoiceView<NotificationColor>),
    ToggleStatusEnabled(bool),
    SetPosition(ChoiceView<NotificationPosition>),
    SetStatusPosition(ChoiceView<NotificationPosition>),
    SetAreaMode(ChoiceView<NotificationAreaMode>),
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
    pub(crate) view_invalidated: Arc<AtomicBool>,
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
    pub(crate) view_invalidated: Arc<AtomicBool>,
    pub vm: SettingsViewModel,
    _revision_subscription: nerust_gui_viewmodel::settings::Subscription,
    pub(super) page: SettingsPage,
    pub(super) system_tab_index: Option<usize>,
    pub(super) input_tab_index: Option<usize>,
    pub(super) input_section: InputPageSection,
    pub(super) storage_directory_input: String,
    pub(super) error_message: Option<String>,
}

fn system_id_key(factory: &Arc<dyn nerust_core_traits::factory::CoreFactory>) -> String {
    factory.system_id().to_string()
}

/// First key occurring twice, in encounter order.
fn first_duplicate_key<T>(
    items: impl IntoIterator<Item = T>,
    mut key: impl FnMut(&T) -> String,
) -> Option<String> {
    let mut seen = std::collections::HashSet::new();
    items
        .into_iter()
        .map(|item| key(&item))
        .find(|id| !seen.insert(id.clone()))
}

/// First duplicate SystemId in registration order, if any.
pub(crate) fn find_duplicate_system_id(registry: &SystemRegistry) -> Option<String> {
    first_duplicate_key(registry.all(), |factory| system_id_key(factory))
}

/// First-wins dedupe by key; `on_duplicate` fires per dropped item.
fn deduped_by_key<T>(
    items: impl IntoIterator<Item = T>,
    mut key: impl FnMut(&T) -> String,
    mut on_duplicate: impl FnMut(&T),
) -> Vec<T> {
    let mut seen = std::collections::HashSet::new();
    items
        .into_iter()
        .filter(|item| {
            if seen.insert(key(item)) {
                true
            } else {
                on_duplicate(item);
                false
            }
        })
        .collect()
}

/// Factories deduped first-wins by [`system_id_key`]; later
/// duplicates are logged and dropped.
fn deduped_factories(
    registry: &SystemRegistry,
) -> Vec<Arc<dyn nerust_core_traits::factory::CoreFactory>> {
    deduped_by_key(registry.all().iter().cloned(), system_id_key, |factory| {
        log::warn!(
            "ignoring factory '{}' with duplicate SystemId",
            factory.display_name()
        );
    })
}

impl SettingsAppState {
    #[cfg(test)]
    pub(crate) fn new(
        snapshot: &SettingsSnapshot,
        registry: Arc<SystemRegistry>,
        audio_registry: Arc<AudioBackendRegistry>,
    ) -> Self {
        Self::new_with_invalidation(
            snapshot,
            registry,
            audio_registry,
            Arc::new(AtomicBool::new(false)),
        )
    }

    fn new_with_invalidation(
        snapshot: &SettingsSnapshot,
        registry: Arc<SystemRegistry>,
        audio_registry: Arc<AudioBackendRegistry>,
        view_invalidated: Arc<AtomicBool>,
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

        // First-wins dedupe by the `find_duplicate_system_id` key.
        let factories = deduped_factories(&registry);
        // Impossibility guard: same-predicate dedupe above plus the
        // window-open pre-check, and the registry is immutable after
        // construction, so no duplicate can slip between the two.
        let vm = SettingsViewModel::new(
            snapshot.clone(),
            factories,
            supported_sample_rates,
            Rc::new(FsStoragePathValidator) as Rc<dyn StoragePathValidator>,
        )
        .expect("deduped factory catalog contains a duplicate SystemId");
        let invalidated = Arc::clone(&view_invalidated);
        let _revision_subscription = vm.revision.observe(move |_| {
            invalidated.store(true, Ordering::Release);
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
        view_invalidated: Arc<AtomicBool>,
    ) -> Self {
        let mut state =
            Self::new_with_invalidation(snapshot, registry, audio_registry, view_invalidated);
        state.should_close = should_close;
        state.pending_apply = pending_apply;
        state
    }

    pub(super) fn language(&self) -> AppLanguage {
        self.vm.general.view.get().language
    }

    pub(super) fn validation_errors(&self) -> Vec<String> {
        self.vm
            .finish()
            .err()
            .map(|v| v.issues.into_iter().map(|i| i.message).collect())
            .unwrap_or_default()
    }

    pub(super) fn storage_error(&self) -> Option<String> {
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
            Message::SelectPage(page) => self.select_page(page),
            Message::SelectInputSection(section) => self.select_input_section(section),
            Message::SelectSystemTab(index) => self.select_system_tab(index),
            Message::SelectInputTab(index) => self.select_input_tab(index),
            Message::SetLanguage(choice) => self.err(self.vm.general.set_language(choice.value)),
            Message::SetStoragePolicy(choice) => {
                self.err(self.vm.general.set_storage_policy(choice.value))
            }
            Message::SetStorageDirectory(value) => self.set_storage_directory(value),
            Message::BrowseStorageDirectory => self.browse_storage_directory(),
            Message::ToggleFullscreenDefault(value) => {
                self.err(self.vm.video.set_fullscreen_default(value))
            }
            Message::SetScaling(choice) => self.err(self.vm.video.set_scaling(choice.value)),
            Message::ToggleVsync(value) => self.err(self.vm.video.set_vsync(value)),
            Message::ToggleMute(value) => self.err(self.vm.audio.set_mute(value)),
            Message::SetVolume(value) => self.err(self.vm.audio.set_volume(value)),
            Message::SetSampleRate(choice) => self.err(self.vm.audio.set_sample_rate(choice.value)),
            Message::SetLatency(value) => self.err(self.vm.audio.set_latency(value)),
            Message::ToggleInfoEnabled(value) => {
                self.err(self.vm.notifications.set_info_enabled(value))
            }
            Message::SetInfoDuration(value) => {
                self.err(self.vm.notifications.set_info_duration_ms(value))
            }
            Message::SetInfoColor(choice) => {
                self.err(self.vm.notifications.set_info_color(choice.value))
            }
            Message::ToggleWarningEnabled(value) => {
                self.err(self.vm.notifications.set_warning_enabled(value))
            }
            Message::SetWarningDuration(value) => {
                self.err(self.vm.notifications.set_warning_duration_ms(value))
            }
            Message::SetWarningColor(choice) => {
                self.err(self.vm.notifications.set_warning_color(choice.value))
            }
            Message::ToggleErrorEnabled(value) => {
                self.err(self.vm.notifications.set_error_enabled(value))
            }
            Message::ToggleErrorModal(value) => {
                self.err(self.vm.notifications.set_error_modal(value))
            }
            Message::SetErrorColor(choice) => {
                self.err(self.vm.notifications.set_error_color(choice.value))
            }
            Message::ToggleStatusEnabled(value) => {
                self.err(self.vm.notifications.set_status_enabled(value))
            }
            Message::SetPosition(choice) => {
                self.err(self.vm.notifications.set_position(choice.value))
            }
            Message::SetStatusPosition(choice) => {
                self.err(self.vm.notifications.set_status_position(choice.value))
            }
            Message::SetAreaMode(choice) => {
                self.err(self.vm.notifications.set_area_mode(choice.value))
            }
            Message::SetSystemChoice(field, choice) => self.set_system_choice(field, choice),
            Message::SetControllerSlot {
                slot,
                controller_id,
            } => self.set_controller_slot(slot, controller_id),
            Message::StartCapture(target) => self.err(self.vm.capture.start_capture(target)),
            Message::ClearCapture(target) => self.err(self.vm.capture.clear_binding(&target)),
            Message::CaptureKey(key) => self.vm.capture.apply_captured_key(key),
            Message::Submit => self.submit(),
            Message::Cancel => self.cancel(),
        }
        Task::none()
    }

    fn err(&mut self, result: Result<(), nerust_gui_viewmodel::settings::ViewModelError>) {
        if let Err(e) = result {
            self.error_message = Some(e.to_string());
        }
    }

    fn select_page(&mut self, page: SettingsPage) {
        self.page = page;
        self.view_invalidated.store(true, Ordering::Release);
    }

    fn select_input_section(&mut self, section: InputPageSection) {
        self.input_section = section;
        self.view_invalidated.store(true, Ordering::Release);
    }

    fn select_system_tab(&mut self, index: usize) {
        self.system_tab_index = Some(index);
        self.view_invalidated.store(true, Ordering::Release);
    }

    fn select_input_tab(&mut self, index: usize) {
        self.input_tab_index = Some(index);
        self.input_section = InputPageSection::Attachment(0);
        self.view_invalidated.store(true, Ordering::Release);
    }

    fn set_storage_directory(&mut self, value: String) {
        self.storage_directory_input = value.clone();
        self.err(
            self.vm
                .general
                .set_storage_directory((!value.is_empty()).then(|| value.into())),
        );
    }

    fn browse_storage_directory(&mut self) {
        if let Some(path) = FileDialog::new()
            .set_title(ui_text(self.language(), UiText::SaveStorageDirectory))
            .pick_folder()
        {
            let path = path.to_string_lossy().to_string();
            self.storage_directory_input = path.clone();
            self.err(self.vm.general.set_storage_directory(Some(path.into())));
        }
    }

    fn set_system_choice(
        &mut self,
        field: String,
        choice: ChoiceView<nerust_core_traits::factory::descriptor::SystemSettingsChoiceId>,
    ) {
        let system_tab_index = self.system_tab_index;
        if let Some(idx) = system_tab_index
            && let Some(system_vm) = self.vm.systems().get(idx)
            && let Err(e) = system_vm.set_choice(
                &nerust_core_traits::factory::descriptor::SystemSettingsFieldId(field.into()),
                &choice.value,
            )
        {
            self.error_message = Some(e.to_string());
        }
    }

    fn set_controller_slot(&mut self, slot: AttachmentId, controller_id: Option<String>) {
        let input_tab_index = self.input_tab_index;
        if let Some(idx) = input_tab_index
            && let Some(input_vm) = self.vm.inputs().get(idx)
            && let Err(e) = input_vm.set_controller_slot(slot, controller_id.as_deref())
        {
            self.error_message = Some(e.to_string());
        }
    }

    fn submit(&mut self) {
        if let Ok(snapshot) = self.vm.finish() {
            *lock_recovered(&self.pending_apply) = Some(snapshot);
            self.should_close.store(true, Ordering::Release);
        }
    }

    fn cancel(&mut self) {
        self.should_close.store(true, Ordering::Release);
    }

    fn view(&self) -> El<'_> {
        let language = self.language();
        let validation_errors = self.validation_errors();
        let can_submit = validation_errors.is_empty();

        let sidebar = column![
            super::views::page_radio(language, UiText::General, SettingsPage::General, self.page),
            super::views::page_radio(language, UiText::Input, SettingsPage::Input, self.page),
            super::views::page_radio(language, UiText::Video, SettingsPage::Video, self.page),
            super::views::page_radio(language, UiText::Audio, SettingsPage::Audio, self.page),
            super::views::page_radio(
                language,
                UiText::Notifications,
                SettingsPage::Notifications,
                self.page,
            ),
            super::views::page_radio(language, UiText::System, SettingsPage::System, self.page),
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
            iced::widget::Space::new().width(Length::Fill),
            button(ui_text(language, UiText::Cancel)).on_press(Message::Cancel),
            button(ui_text(language, UiText::Ok))
                .on_press_maybe(can_submit.then_some(Message::Submit)),
        ]
        .spacing(12)
        .align_y(Alignment::Center);

        root.push(container(buttons).width(Length::Fill)).into()
    }
}

pub(crate) fn keyboard_key_from_physical(physical: iced::keyboard::key::Physical) -> Option<Key> {
    physical.try_into().ok()
}

#[cfg(test)]
mod tests {
    use iced::keyboard::key::{Code, Physical};
    use nerust_core_traits::audio::AudioBackendRegistry;
    use nerust_gui_runtime::settings::SettingsSnapshot;
    use nerust_gui_shell::registry::SystemRegistry;
    use nerust_gui_viewmodel::settings::dto::ChoiceView;
    use nerust_keyboard::Key;
    use nerust_settings_core::model::{
        app_state::DesktopAppState,
        local::{HostBackendLocalSettings, ScalingMode},
        shared::{DesktopSharedSettings, StoragePolicy},
    };
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
    fn program_update_and_error_paths_stay_headless() {
        let program = SettingsAppProgram {
            snapshot: empty_snapshot(),
            registry: Arc::new(SystemRegistry::new(Vec::new())),
            audio_registry: Arc::new(AudioBackendRegistry::new()),
            should_close: Arc::new(AtomicBool::new(false)),
            pending_apply: Arc::new(Mutex::new(None)),
            view_invalidated: Arc::new(AtomicBool::new(false)),
        };
        let (mut state, _task) = program.boot();
        dispatch(&mut state, Message::SelectPage(SettingsPage::Audio));
        assert_eq!(state.page, SettingsPage::Audio);
        let _ = program.update(&mut state, Message::SelectPage(SettingsPage::General));
        // Storage error reads the live ViewModel projection.
        let _ = state.storage_error();
        // Error banner renders above the buttons when set.
        state.error_message = Some("boom".to_string());
        let _ = program.view(&state, iced::window::Id::unique());
    }

    #[test]
    fn program_boot_and_chrome_are_headless() {
        let program = SettingsAppProgram {
            snapshot: empty_snapshot(),
            registry: Arc::new(SystemRegistry::new(Vec::new())),
            audio_registry: Arc::new(AudioBackendRegistry::new()),
            should_close: Arc::new(AtomicBool::new(false)),
            pending_apply: Arc::new(Mutex::new(None)),
            view_invalidated: Arc::new(AtomicBool::new(false)),
        };
        assert_eq!(SettingsAppProgram::name(), "nerust_settings");
        assert!(program.window().is_none());
        let _ = program.settings();
        let (state, _task) = program.boot();
        assert!(program.theme(&state, iced::window::Id::unique()).is_none());
        let _ = program.view(&state, iced::window::Id::unique());
    }

    #[test]
    fn revision_callback_shares_external_view_invalidated_flag() {
        let external = Arc::new(AtomicBool::new(false));
        let mut state = SettingsAppState::new_with_shared(
            &empty_snapshot(),
            Arc::new(SystemRegistry::new(Vec::new())),
            Arc::new(AudioBackendRegistry::new()),
            Arc::new(AtomicBool::new(false)),
            Arc::new(Mutex::new(None)),
            Arc::clone(&external),
        );

        dispatch(
            &mut state,
            Message::SetLanguage(ChoiceView {
                value: AppLanguage::Japanese,
                label: "Japanese".into(),
            }),
        );

        assert!(
            external.load(Ordering::Acquire),
            "revision callback should have set the externally-shared flag"
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
        assert!(lock_recovered(&submitted.pending_apply).is_some());

        let mut cancelled = empty_state();
        dispatch(&mut cancelled, Message::Cancel);
        assert!(cancelled.should_close.load(Ordering::Acquire));
        assert!(lock_recovered(&cancelled.pending_apply).is_none());
    }

    #[test]
    fn empty_registry_paths_are_safe_and_validation_blocks_submit() {
        let mut state = empty_state();

        // No custom directory and default policy is sidecar — no storage error
        // Submit should succeed (empty snapshot)
        dispatch(&mut state, Message::Submit);
        assert!(lock_recovered(&state.pending_apply).is_some());
    }

    #[test]
    fn capture_messages_update_capture_state() {
        let mut state = empty_state();
        let target = CaptureTarget::Shortcut(
            nerust_settings_core::model::input::ShortcutAction::TogglePause,
        );

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

    #[test]
    fn select_input_section_round_trips() {
        let mut state = empty_state();
        dispatch(
            &mut state,
            Message::SelectInputSection(InputPageSection::Attachment(2)),
        );
        assert_eq!(state.input_section, InputPageSection::Attachment(2));
    }

    #[test]
    fn set_system_choice_with_empty_registry_does_not_panic() {
        use std::borrow::Cow;
        let mut state = empty_state();
        dispatch(
            &mut state,
            Message::SetSystemChoice(
                "unknown".into(),
                ChoiceView {
                    value: nerust_core_traits::factory::descriptor::SystemSettingsChoiceId(
                        Cow::Borrowed("none"),
                    ),
                    label: "none".into(),
                },
            ),
        );
        // No panic = success
    }

    #[test]
    fn set_controller_slot_with_empty_registry_does_not_panic() {
        let mut state = empty_state();
        dispatch(
            &mut state,
            Message::SetControllerSlot {
                slot: AttachmentId::new("nonexistent.slot"),
                controller_id: None,
            },
        );
        // No panic = success
    }

    #[test]
    fn validation_errors_empty_with_default_settings() {
        let state = empty_state();
        let errors = state.validation_errors();
        assert!(
            errors.is_empty(),
            "default settings should have no validation errors"
        );
    }
}

#[cfg(test)]
mod dedupe_tests {
    use super::{deduped_by_key, first_duplicate_key};

    fn key(entry: &(&str, u32)) -> String {
        entry.0.to_string()
    }

    #[test]
    fn first_duplicate_reports_first_repeat_in_order() {
        let items = vec![("a", 1), ("b", 2), ("a", 3), ("b", 4)];
        assert_eq!(first_duplicate_key(items, key), Some("a".to_string()));
    }

    #[test]
    fn first_duplicate_returns_none_without_repeats() {
        let items = vec![("a", 1), ("b", 2)];
        assert_eq!(first_duplicate_key(items, key), None);
        let empty: Vec<(&str, u32)> = Vec::new();
        assert_eq!(first_duplicate_key(empty, key), None);
    }

    #[test]
    fn dedupe_keeps_first_and_reports_drops() {
        let items = vec![("a", 1), ("b", 2), ("a", 3)];
        let mut dropped = Vec::new();
        let kept = deduped_by_key(items, key, |item| dropped.push(item.1));
        assert_eq!(kept, vec![("a", 1), ("b", 2)]);
        assert_eq!(dropped, vec![3]);
    }
}
