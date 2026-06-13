use nerust_gui_runtime::settings::SettingsSnapshot;
use nerust_gui_runtime::slots::slot_label;
use nerust_gui_session::commands::SessionCommand;
use nerust_persistence::model::StateSlotSummary;
use std::sync::mpsc;
use tao::window::Window as TaoWindow;

#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub(crate) enum MenuCommand {
    Open,
    Settings,
    Session(SessionCommand),
    Quit,
}

#[derive(Debug)]
pub(crate) enum UserEvent {
    Menu(MenuCommand),
    ApplySettings {
        snapshot: SettingsSnapshot,
        reply: mpsc::Sender<Result<(), String>>,
    },
    SettingsClosed,
}

#[cfg(any(
    target_os = "linux",
    target_os = "dragonfly",
    target_os = "freebsd",
    target_os = "netbsd",
    target_os = "openbsd",
    target_os = "macos",
    target_os = "windows"
))]
pub(crate) mod imp {
    use super::{MenuCommand, SessionCommand, StateSlotSummary, TaoWindow, UserEvent, slot_label};
    #[cfg(any(
        target_os = "linux",
        target_os = "dragonfly",
        target_os = "freebsd",
        target_os = "netbsd",
        target_os = "openbsd"
    ))]
    use gtk::prelude::WidgetExt;
    use muda::{Menu, MenuEvent, MenuId, MenuItem, Submenu};
    use nerust_contract_settings::language::AppLanguage;
    use nerust_gui_shell::settings::i18n::{UiText, text};
    use std::sync::{Arc, RwLock};
    use tao::event_loop::EventLoopProxy;
    #[cfg(target_os = "macos")]
    use tao::platform::macos::WindowExtMacOS;
    #[cfg(any(
        target_os = "linux",
        target_os = "dragonfly",
        target_os = "freebsd",
        target_os = "netbsd",
        target_os = "openbsd"
    ))]
    use tao::platform::unix::WindowExtUnix;
    #[cfg(target_os = "windows")]
    use tao::platform::windows::WindowExtWindows;

    pub(crate) struct AppMenu {
        menu_bar: Menu,
        file_menu: Submenu,
        emulation_menu: Submenu,
        state_menu: Submenu,
        open: MenuItem,
        settings: MenuItem,
        pause: MenuItem,
        resume: MenuItem,
        reset: MenuItem,
        quit: MenuItem,
        create_slot: MenuItem,
        save_active: MenuItem,
        load_active: MenuItem,
        active_slot_menu: Submenu,
        save_slot_menu: Submenu,
        load_slot_menu: Submenu,
        delete_slot_menu: Submenu,
        dynamic_commands: Arc<RwLock<Vec<(MenuId, SessionCommand)>>>,
    }

    impl AppMenu {
        pub(crate) fn new(proxy: EventLoopProxy<UserEvent>) -> Self {
            let menu_bar = Menu::new();
            let file_menu = Submenu::new("File", true);
            let emulation_menu = Submenu::new("Emulation", true);
            let state_menu = Submenu::new("Save States", true);
            let active_slot_menu = Submenu::new("Select Active Slot", true);
            let save_slot_menu = Submenu::new("Save Slot", true);
            let load_slot_menu = Submenu::new("Load Slot", true);
            let delete_slot_menu = Submenu::new("Delete Slot", true);

            #[cfg(target_os = "macos")]
            {
                let app_menu = Submenu::new("App", true);
                app_menu
                    .append_items(&[
                        &muda::PredefinedMenuItem::about(None, None),
                        &muda::PredefinedMenuItem::separator(),
                        &muda::PredefinedMenuItem::services(None),
                        &muda::PredefinedMenuItem::separator(),
                        &muda::PredefinedMenuItem::hide(None),
                        &muda::PredefinedMenuItem::hide_others(None),
                        &muda::PredefinedMenuItem::show_all(None),
                        &muda::PredefinedMenuItem::separator(),
                        &muda::PredefinedMenuItem::quit(None),
                    ])
                    .unwrap();
                menu_bar.append(&app_menu).unwrap();
            }

            let open = MenuItem::new("Open ROM...", true, None);
            let settings = MenuItem::new("Settings...", true, None);
            let pause = MenuItem::new("Pause", true, None);
            let resume = MenuItem::new("Resume", false, None);
            let reset = MenuItem::new("Reset", true, None);
            let quit = MenuItem::new("Quit", true, None);
            let create_slot = MenuItem::new("Create New Slot", true, None);
            let save_active = MenuItem::new("Save Active Slot (F5)", true, None);
            let load_active = MenuItem::new("Load Active Slot (F8)", false, None);

            let open_id = open.id().clone();
            let settings_id = settings.id().clone();
            let pause_id = pause.id().clone();
            let resume_id = resume.id().clone();
            let reset_id = reset.id().clone();
            let quit_id = quit.id().clone();
            let create_slot_id = create_slot.id().clone();
            let save_active_id = save_active.id().clone();
            let load_active_id = load_active.id().clone();
            let dynamic_commands = Arc::new(RwLock::new(Vec::<(MenuId, SessionCommand)>::new()));
            let dynamic_commands_handler = dynamic_commands.clone();

            file_menu.append(&open).unwrap();
            file_menu.append(&settings).unwrap();
            file_menu.append(&quit).unwrap();
            state_menu.append(&create_slot).unwrap();
            state_menu.append(&save_active).unwrap();
            state_menu.append(&load_active).unwrap();
            state_menu.append(&active_slot_menu).unwrap();
            state_menu.append(&save_slot_menu).unwrap();
            state_menu.append(&load_slot_menu).unwrap();
            state_menu.append(&delete_slot_menu).unwrap();
            emulation_menu.append(&pause).unwrap();
            emulation_menu.append(&resume).unwrap();
            emulation_menu.append(&reset).unwrap();
            emulation_menu.append(&state_menu).unwrap();

            menu_bar.append(&file_menu).unwrap();
            menu_bar.append(&emulation_menu).unwrap();

            MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
                let command = if event.id() == &open_id {
                    Some(MenuCommand::Open)
                } else if event.id() == &settings_id {
                    Some(MenuCommand::Settings)
                } else if event.id() == &pause_id {
                    Some(MenuCommand::Session(SessionCommand::Pause))
                } else if event.id() == &resume_id {
                    Some(MenuCommand::Session(SessionCommand::Resume))
                } else if event.id() == &reset_id {
                    Some(MenuCommand::Session(SessionCommand::Reset))
                } else if event.id() == &quit_id {
                    Some(MenuCommand::Quit)
                } else if event.id() == &create_slot_id {
                    Some(MenuCommand::Session(SessionCommand::CreateSlot))
                } else if event.id() == &save_active_id {
                    Some(MenuCommand::Session(SessionCommand::SaveActiveSlotOrNew))
                } else if event.id() == &load_active_id {
                    Some(MenuCommand::Session(SessionCommand::LoadActiveSlot))
                } else {
                    dynamic_commands_handler
                        .read()
                        .unwrap_or_else(|err| err.into_inner())
                        .iter()
                        .find_map(|(id, command)| {
                            (event.id() == id).then_some(MenuCommand::Session(*command))
                        })
                };
                if let Some(command) = command {
                    let _ = proxy.send_event(UserEvent::Menu(command));
                }
            }));

            Self {
                menu_bar,
                file_menu,
                emulation_menu,
                state_menu,
                open,
                settings,
                pause,
                resume,
                reset,
                quit,
                create_slot,
                save_active,
                load_active,
                active_slot_menu,
                save_slot_menu,
                load_slot_menu,
                delete_slot_menu,
                dynamic_commands,
            }
        }

        pub(crate) fn init_for_window(&self, window: &TaoWindow) {
            #[cfg(target_os = "windows")]
            unsafe {
                self.menu_bar.init_for_hwnd(window.hwnd() as _).unwrap();
            }

            #[cfg(any(
                target_os = "linux",
                target_os = "dragonfly",
                target_os = "freebsd",
                target_os = "netbsd",
                target_os = "openbsd"
            ))]
            {
                self.menu_bar
                    .init_for_gtk_window(window.gtk_window(), window.default_vbox())
                    .unwrap();
                window.gtk_window().show_all();
            }

            #[cfg(target_os = "macos")]
            {
                let _ = window.ns_view();
                self.menu_bar.init_for_nsapp();
            }
        }

        pub(crate) fn update(
            &mut self,
            loaded: bool,
            paused: bool,
            slots: &[StateSlotSummary],
            active_slot: Option<u64>,
            settings_open: bool,
            language: AppLanguage,
        ) {
            self.update_labels(language);
            self.open.set_enabled(!settings_open);
            self.settings.set_enabled(!settings_open);
            self.pause.set_enabled(!settings_open && loaded && !paused);
            self.resume.set_enabled(!settings_open && loaded && paused);
            self.create_slot.set_enabled(!settings_open && loaded);
            self.save_active.set_enabled(!settings_open && loaded);
            self.load_active
                .set_enabled(!settings_open && loaded && active_slot.is_some());
            self.rebuild_dynamic_slot_menus(slots, active_slot);
        }

        pub(crate) fn clear_event_handler(&self) {
            MenuEvent::set_event_handler::<fn(MenuEvent)>(None);
        }

        fn rebuild_dynamic_slot_menus(
            &mut self,
            slots: &[StateSlotSummary],
            active_slot: Option<u64>,
        ) {
            clear_submenu(&self.active_slot_menu);
            clear_submenu(&self.save_slot_menu);
            clear_submenu(&self.load_slot_menu);
            clear_submenu(&self.delete_slot_menu);
            let mut commands = Vec::new();
            for slot in slots {
                let select_item = MenuItem::new(slot_label(slot, active_slot), true, None);
                commands.push((
                    select_item.id().clone(),
                    SessionCommand::SelectActiveSlot(slot.slot_id),
                ));
                self.active_slot_menu.append(&select_item).unwrap();

                let save_item = MenuItem::new(slot_label(slot, active_slot), true, None);
                commands.push((
                    save_item.id().clone(),
                    SessionCommand::SaveSlot(slot.slot_id),
                ));
                self.save_slot_menu.append(&save_item).unwrap();

                let load_item = MenuItem::new(slot_label(slot, active_slot), true, None);
                commands.push((
                    load_item.id().clone(),
                    SessionCommand::LoadSlot(slot.slot_id),
                ));
                self.load_slot_menu.append(&load_item).unwrap();

                let delete_item = MenuItem::new(slot_label(slot, active_slot), true, None);
                commands.push((
                    delete_item.id().clone(),
                    SessionCommand::DeleteSlot(slot.slot_id),
                ));
                self.delete_slot_menu.append(&delete_item).unwrap();
            }
            *self
                .dynamic_commands
                .write()
                .unwrap_or_else(|err| err.into_inner()) = commands;
        }

        fn update_labels(&self, language: AppLanguage) {
            self.file_menu.set_text(text(language, UiText::File));
            self.emulation_menu
                .set_text(text(language, UiText::Emulation));
            self.state_menu.set_text(text(language, UiText::SaveStates));
            self.active_slot_menu
                .set_text(text(language, UiText::SelectActiveSlot));
            self.save_slot_menu
                .set_text(text(language, UiText::SaveSlot));
            self.load_slot_menu
                .set_text(text(language, UiText::LoadSlot));
            self.delete_slot_menu
                .set_text(text(language, UiText::DeleteSlot));

            self.open
                .set_text(format!("{}...", text(language, UiText::Open)));
            self.settings
                .set_text(format!("{}...", text(language, UiText::Settings)));
            self.pause.set_text(text(language, UiText::Pause));
            self.resume.set_text(text(language, UiText::Resume));
            self.reset.set_text(text(language, UiText::Reset));
            self.quit.set_text(text(language, UiText::Quit));
            self.create_slot
                .set_text(text(language, UiText::CreateSaveSlot));
            self.save_active.set_text(menu_label_with_shortcut(
                text(language, UiText::SaveActiveSlot),
                "F5",
            ));
            self.load_active.set_text(menu_label_with_shortcut(
                text(language, UiText::LoadActiveSlot),
                "F8",
            ));
        }
    }

    fn clear_submenu(menu: &Submenu) {
        while !menu.items().is_empty() {
            let _ = menu.remove_at(0);
        }
    }

    fn menu_label_with_shortcut(label: &str, shortcut: &str) -> String {
        format!("{label} ({shortcut})")
    }
}

#[cfg(not(any(
    target_os = "linux",
    target_os = "dragonfly",
    target_os = "freebsd",
    target_os = "netbsd",
    target_os = "openbsd",
    target_os = "macos",
    target_os = "windows"
)))]
pub(crate) mod imp {
    use super::{StateSlotSummary, TaoWindow, UserEvent};
    use nerust_contract_settings::language::AppLanguage;
    use tao::event_loop::EventLoopProxy;

    pub(crate) struct AppMenu;

    impl AppMenu {
        pub(crate) fn new(_proxy: EventLoopProxy<UserEvent>) -> Self {
            Self
        }

        pub(crate) fn init_for_window(&self, _window: &TaoWindow) {}

        pub(crate) fn update(
            &mut self,
            _loaded: bool,
            _paused: bool,
            _slots: &[StateSlotSummary],
            _active_slot: Option<u64>,
            _settings_open: bool,
            _language: AppLanguage,
        ) {
        }

        pub(crate) fn clear_event_handler(&self) {}
    }
}
