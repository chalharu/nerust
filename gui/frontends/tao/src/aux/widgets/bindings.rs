use std::{
    cell::RefCell,
    ops::{Deref, DerefMut},
    sync::{Arc, Mutex, atomic::AtomicBool},
};

use crate::aux::window::WidgetBinding;
use iced::{Task, Theme};
#[cfg(test)]
use nerust_core_traits::audio::AudioBackendRegistry;
use nerust_frontend_host::settings::{
    ControlInput, SettingsControlId, SettingsController, SettingsEffect, SettingsEvent,
};
use nerust_gui_runtime::settings::SettingsSnapshot;
#[cfg(test)]
use nerust_gui_shell::registry::SystemRegistry;
use nerust_keyboard::Key;

use crate::convert::default_font;

pub(crate) type El<'a> = iced::Element<'a, Message, iced::Theme, iced_tiny_skia::Renderer>;

/// Native callbacks retain identity and input, never a snapshot's bound events.
#[derive(Debug, Clone)]
pub(crate) enum Message {
    Input(SettingsControlId, ControlInput),
    CaptureKey(Key),
}

pub(crate) struct SettingsBinding {
    // Stage ownership, not a second draft; boot moves this exact controller.
    initial_state: RefCell<Option<Box<SettingsAppState>>>,
    pub(crate) should_close: Arc<AtomicBool>,
    pub(crate) pending_apply: Arc<Mutex<Option<SettingsSnapshot>>>,
    pub(crate) view_invalidated: Arc<AtomicBool>,
}

impl SettingsBinding {
    pub(crate) fn from_controller(controller: SettingsController) -> Self {
        Self {
            should_close: controller.should_close.clone(),
            pending_apply: controller.pending_apply.clone(),
            view_invalidated: controller.view_invalidated.clone(),
            initial_state: RefCell::new(Some(Box::new(SettingsAppState { controller }))),
        }
    }

    #[cfg(test)]
    pub(crate) fn new(
        snapshot: SettingsSnapshot,
        registry: Arc<SystemRegistry>,
        audio_registry: Arc<AudioBackendRegistry>,
        should_close: Arc<AtomicBool>,
        pending_apply: Arc<Mutex<Option<SettingsSnapshot>>>,
        view_invalidated: Arc<AtomicBool>,
    ) -> Self {
        let state = SettingsAppState::new_with_shared(
            &snapshot,
            registry,
            audio_registry,
            should_close.clone(),
            pending_apply.clone(),
            view_invalidated.clone(),
        );
        Self {
            initial_state: RefCell::new(Some(Box::new(state))),
            should_close,
            pending_apply,
            view_invalidated,
        }
    }

    #[cfg(test)]
    pub(crate) fn initial_window(&self) -> nerust_frontend_host::window::Window {
        self.initial_state
            .borrow()
            .as_ref()
            .expect("not booted")
            .controller
            .window()
    }
}

impl WidgetBinding for SettingsBinding {
    type State = SettingsAppState;
    type Message = Message;
    type Theme = Theme;
    type Renderer = iced_tiny_skia::Renderer;
    type Executor = iced_winit::futures::backend::default::Executor;

    const KIND: crate::aux::focus::AuxKind = crate::aux::focus::AuxKind::Settings;

    fn settings(&self) -> iced::Settings {
        iced::Settings {
            default_font: default_font(),
            default_text_size: iced::Pixels(16.0),
            ..Default::default()
        }
    }

    fn boot(&self) -> (Self::State, Task<Self::Message>) {
        let state = self
            .initial_state
            .borrow_mut()
            .take()
            .expect("settings boot once");
        (*state, Task::none())
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

    fn presentation(&self, state: &Self::State) -> Option<nerust_frontend_host::window::Window> {
        Some(state.controller.window())
    }

    fn settings_view<'a>(
        &self,
        _state: &'a Self::State,
        screen: &nerust_frontend_host::settings::SettingsScreen,
        _id: iced::window::Id,
    ) -> El<'a> {
        super::controls::build_screen(screen)
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
    #[cfg(test)]
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
        // Resolve once, release the snapshot, then apply context and edit in order.
        let events = match message {
            Message::Input(id, input) => self.controller.screen().bind(&id, input),
            Message::CaptureKey(key) => self.controller.screen().bind_capture_key(key),
        };
        for event in events.into_iter().flatten() {
            if let Some(SettingsEffect::BrowseStorageDirectory { title }) =
                self.controller.update(event)
            {
                let path = rfd::FileDialog::new().set_title(title).pick_folder();
                self.controller
                    .update(SettingsEvent::StorageDirectorySelected(path));
            }
        }
        Task::none()
    }

    fn view(&self) -> El<'_> {
        super::controls::build_screen(&self.controller.screen())
    }
}

pub(crate) fn keyboard_key_from_physical(physical: iced::keyboard::key::Physical) -> Option<Key> {
    physical.try_into().ok()
}

#[cfg(test)]
mod tests {
    use super::SettingsBinding as SettingsAppProgram;
    use iced::keyboard::key::{Code, Physical};
    use nerust_core_traits::audio::AudioBackendRegistry;
    use nerust_frontend_host::settings::{SettingsControlId as Id, SettingsPage};
    use nerust_gui_runtime::settings::SettingsSnapshot;
    use nerust_gui_shell::registry::SystemRegistry;
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

    #[test]
    fn common_window_program_moves_the_prepared_controller_and_binds_latest_screen() {
        use crate::aux::window::WindowProgram;
        use iced_winit::program::Program;

        let binding = SettingsBinding::new(
            empty_snapshot(),
            Arc::new(SystemRegistry::new(Vec::new())),
            Arc::new(AudioBackendRegistry::new()),
            Default::default(),
            Default::default(),
            Default::default(),
        );
        binding
            .initial_state
            .borrow_mut()
            .as_mut()
            .unwrap()
            .error_message = Some("prepared".into());
        let program = WindowProgram {
            window: binding.initial_window(),
            program: binding,
        };
        let (mut state, _) = program.boot();
        assert!(program.program.initial_state.borrow().is_none());
        assert_eq!(state.error_message.as_deref(), Some("prepared"));
        assert!(Arc::ptr_eq(
            &state.pending_apply,
            &program.program.pending_apply
        ));
        state.vm.notifications.set_info_enabled(true).unwrap();
        let old_callback = Message::Input(Id::InfoDuration, ControlInput::Number(2000));
        state.vm.notifications.set_info_enabled(false).unwrap();
        let revision = state.vm.revision.get();
        drop(program.update(&mut state, old_callback));
        assert_eq!(state.vm.revision.get(), revision);
        assert_eq!(state.error_message.as_deref(), Some("prepared"));
        let id = iced::window::Id::unique();
        assert_eq!(program.title(&state, id), state.screen().title);
        drop(program.view(&state, id));
    }

    #[test]
    fn prepared_window_contains_the_controller_moved_into_boot() {
        use nerust_frontend_host::window::WidgetContents;

        let program = SettingsAppProgram::new(
            empty_snapshot(),
            Arc::new(SystemRegistry::new(Vec::new())),
            Arc::new(AudioBackendRegistry::new()),
            Default::default(),
            Default::default(),
            Default::default(),
        );
        {
            let mut initial = program.initial_state.borrow_mut();
            let state = initial.as_mut().unwrap();
            state.page = SettingsPage::Notifications;
            state.error_message = Some("prepared draft".into());
        }
        let window = program.initial_window();
        let WidgetContents::Settings(screen) = &window.contents.contents else {
            panic!("real settings screen must exist before native creation");
        };
        assert_eq!(screen.message.as_deref(), Some("prepared draft"));
        let (state, _) = program.boot();
        assert!(program.initial_state.borrow().is_none());
        assert_eq!(state.page, SettingsPage::Notifications);
        assert_eq!(state.error_message.as_deref(), Some("prepared draft"));
        assert_eq!(state.window().title, window.title);
    }

    fn dispatch(state: &mut SettingsAppState, message: Message) {
        drop(state.update(message));
    }

    #[test]
    fn status_area_message_reaches_viewmodel() {
        use nerust_settings_core::model::local::NotificationAreaMode;
        let (mut state, _task) = SettingsAppProgram::new(
            empty_snapshot(),
            Arc::new(SystemRegistry::new(Vec::new())),
            Arc::new(AudioBackendRegistry::new()),
            Arc::new(AtomicBool::new(false)),
            Arc::new(Mutex::new(None)),
            Arc::new(AtomicBool::new(false)),
        )
        .boot();
        state.vm.notifications.set_status_enabled(true).unwrap();
        state
            .vm
            .notifications
            .set_position(nerust_settings_core::model::local::NotificationPosition::TopLeft)
            .unwrap();
        state
            .vm
            .notifications
            .set_status_position(
                nerust_settings_core::model::local::NotificationPosition::BottomLeft,
            )
            .unwrap();
        dispatch(
            &mut state,
            Message::Input(Id::StatusAreaMode, ControlInput::Select(1)),
        );
        assert_eq!(
            state.vm.notifications.view.get().status_area_mode,
            NotificationAreaMode::Strip
        );
        // The notifications page (with the new Status area row)
        // builds without panicking.
        state.page = SettingsPage::Notifications;
        let _ = state.view();
    }

    #[test]
    fn program_update_and_error_paths_stay_headless() {
        let program = SettingsAppProgram::new(
            empty_snapshot(),
            Arc::new(SystemRegistry::new(Vec::new())),
            Arc::new(AudioBackendRegistry::new()),
            Arc::new(AtomicBool::new(false)),
            Arc::new(Mutex::new(None)),
            Arc::new(AtomicBool::new(false)),
        );
        let (mut state, _task) = program.boot();
        dispatch(
            &mut state,
            Message::Input(Id::Page(SettingsPage::Audio), ControlInput::Activate),
        );
        assert_eq!(state.page, SettingsPage::Audio);
        let _ = program.update(
            &mut state,
            Message::Input(Id::Page(SettingsPage::General), ControlInput::Activate),
        );
        // Storage error reads the live ViewModel projection.
        let _ = state.validation_banner();
        // Error banner renders above the buttons when set.
        state.error_message = Some("boom".to_string());
        let _ = program.view(&state, iced::window::Id::unique());
    }

    #[test]
    fn program_boot_and_chrome_are_headless() {
        let program = SettingsAppProgram::new(
            empty_snapshot(),
            Arc::new(SystemRegistry::new(Vec::new())),
            Arc::new(AudioBackendRegistry::new()),
            Arc::new(AtomicBool::new(false)),
            Arc::new(Mutex::new(None)),
            Arc::new(AtomicBool::new(false)),
        );
        let _ = program.settings();
        let (state, _task) = program.boot();
        assert!(Arc::ptr_eq(&state.should_close, &program.should_close));
        assert!(Arc::ptr_eq(&state.pending_apply, &program.pending_apply));
        assert!(Arc::ptr_eq(
            &state.view_invalidated,
            &program.view_invalidated
        ));
        assert!(program.theme(&state, iced::window::Id::unique()).is_none());
        assert_eq!(
            program.presentation(&state).unwrap().title,
            state.screen().title
        );
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
