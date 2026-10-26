use std::{
    ops::{Deref, DerefMut},
    sync::{Arc, Mutex, atomic::AtomicBool},
};

use iced::{
    Length, Task, Theme,
    alignment::Alignment,
    widget::{button, column, container, row, scrollable, text},
};
use iced_winit::program::Program;
use nerust_core_traits::audio::AudioBackendRegistry;
pub(crate) use nerust_frontend_host::settings::{
    InputPageSection, InputTab, SettingsEvent as Message, SettingsPage, find_duplicate_system_id,
};
use nerust_frontend_host::settings::{SettingsController, SettingsEffect};
use nerust_gui_runtime::settings::SettingsSnapshot;
use nerust_gui_shell::registry::SystemRegistry;
use nerust_keyboard::Key;
use nerust_settings_core::i18n::{UiText, text as ui_text};

use crate::convert::default_font;

pub(super) type El<'a> = iced::Element<'a, Message, iced::Theme, iced_tiny_skia::Renderer>;

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

/// Native view wrapper; the common controller owns all dialog state.
pub(crate) struct SettingsAppState {
    controller: SettingsController,
}

impl Deref for SettingsAppState {
    type Target = SettingsController;

    fn deref(&self) -> &Self::Target {
        &self.controller
    }
}

impl DerefMut for SettingsAppState {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.controller
    }
}

impl SettingsAppState {
    pub(crate) fn new_with_shared(
        snapshot: &SettingsSnapshot,
        registry: Arc<SystemRegistry>,
        audio_registry: Arc<AudioBackendRegistry>,
        should_close: Arc<AtomicBool>,
        pending_apply: Arc<Mutex<Option<SettingsSnapshot>>>,
        view_invalidated: Arc<AtomicBool>,
    ) -> Self {
        Self {
            controller: SettingsController::new_with_shared(
                snapshot,
                registry,
                audio_registry,
                should_close,
                pending_apply,
                view_invalidated,
            ),
        }
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        if let Some(SettingsEffect::BrowseStorageDirectory { title }) =
            self.controller.update(message)
        {
            let path = rfd::FileDialog::new().set_title(title).pick_folder();
            self.controller
                .update(Message::StorageDirectorySelected(path));
        }
        Task::none()
    }

    fn view(&self) -> El<'_> {
        let language = self.language();
        let banner = self.validation_banner();
        let can_submit = banner.can_submit;

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
        } else if let Some(headline) = banner.headline.as_ref() {
            root = root.push(text(headline.clone()));
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
        app_state::DesktopAppState, local::HostBackendLocalSettings, shared::DesktopSharedSettings,
    };
    use std::sync::Arc;

    use super::*;

    fn empty_snapshot() -> SettingsSnapshot {
        SettingsSnapshot {
            shared: DesktopSharedSettings::default(),
            local: HostBackendLocalSettings::default(),
            app_state: DesktopAppState::default(),
        }
    }

    fn dispatch(state: &mut SettingsAppState, message: Message) {
        drop(state.update(message));
    }

    #[test]
    fn status_area_message_reaches_viewmodel() {
        use nerust_settings_core::model::local::NotificationAreaMode;
        let (mut state, _task) = SettingsAppProgram {
            snapshot: empty_snapshot(),
            registry: Arc::new(SystemRegistry::new(Vec::new())),
            audio_registry: Arc::new(AudioBackendRegistry::new()),
            should_close: Arc::new(AtomicBool::new(false)),
            pending_apply: Arc::new(Mutex::new(None)),
            view_invalidated: Arc::new(AtomicBool::new(false)),
        }
        .boot();
        dispatch(
            &mut state,
            Message::SetStatusAreaMode(ChoiceView {
                value: NotificationAreaMode::Strip,
                label: String::new(),
            }),
        );
        assert_eq!(
            state.vm.notifications.view.get().status_area_mode,
            NotificationAreaMode::Strip
        );
        // The notifications page (with the new Status area row)
        // builds without panicking.
        state.page = SettingsPage::Notifications;
        let _ = state.page_content();
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
        let _ = state.validation_banner();
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
        assert!(Arc::ptr_eq(&state.should_close, &program.should_close));
        assert!(Arc::ptr_eq(&state.pending_apply, &program.pending_apply));
        assert!(Arc::ptr_eq(
            &state.view_invalidated,
            &program.view_invalidated
        ));
        assert!(program.theme(&state, iced::window::Id::unique()).is_none());
        let _ = program.view(&state, iced::window::Id::unique());
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
}
