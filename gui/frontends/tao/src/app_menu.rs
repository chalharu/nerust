use nerust_gui_runtime::slots::slot_label;
use nerust_gui_shell::session::commands::SessionCommand;
use nerust_persistence::model::StateSlotSummary;
use nerust_settings_core::model::input::ShortcutBinding;
use nerust_settings_core::model::language::AppLanguage;
use tao::window::Window as TaoWindow;

#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub(crate) enum MenuCommand {
    Open,
    Close,
    Settings,
    Debugger,
    Session(SessionCommand),
    Quit,
}

#[derive(Debug)]
pub(crate) enum UserEvent {
    Menu(MenuCommand),
    /// Periodic live-follow pulse for the PPU viewer (see
    /// `WindowRuntime::run`): user events always wake a `Wait` loop,
    /// unlike timer-based waits. The handler no-ops unless a running
    /// core has the viewer open.
    LiveTick,
}

/// One snapshot for a menu sync: a struct (not a long argument
/// list) so adding a field never trips the arity lint. Shared by
/// the real menu and the headless stub below.
pub(crate) struct MenuSync<'a> {
    pub(crate) loaded: bool,
    pub(crate) paused: bool,
    pub(crate) slots: &'a [StateSlotSummary],
    pub(crate) active_slot: Option<u64>,
    pub(crate) settings_open: bool,
    pub(crate) language: AppLanguage,
    pub(crate) shortcuts: &'a [ShortcutBinding],
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
    use std::sync::{Arc, RwLock};

    #[cfg(any(
        target_os = "linux",
        target_os = "dragonfly",
        target_os = "freebsd",
        target_os = "netbsd",
        target_os = "openbsd"
    ))]
    use gtk::prelude::WidgetExt;
    use muda::{Menu, MenuEvent, MenuId, MenuItem, Submenu};
    use nerust_settings_core::i18n::{UiText, menu_label_with_shortcut, text};
    use nerust_settings_core::model::input::{ShortcutAction, ShortcutBinding, shortcut_key_label};
    use nerust_settings_core::model::language::AppLanguage;
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

    use super::{
        MenuCommand, MenuSync, SessionCommand, StateSlotSummary, TaoWindow, UserEvent, slot_label,
    };

    pub(crate) struct AppMenu {
        menu_bar: Menu,
        file_menu: Submenu,
        emulation_menu: Submenu,
        state_menu: Submenu,
        open: MenuItem,
        close: MenuItem,
        settings: MenuItem,
        debugger: MenuItem,
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
            let close = MenuItem::new("Close", true, None);
            let settings = MenuItem::new("Settings...", true, None);
            // Shortcut hints are filled by `update` from the live
            // bindings; the constructor keeps plain labels so a
            // rebound key never flashes the stale default first.
            let debugger = MenuItem::new("Open Debugger", true, None);
            let pause = MenuItem::new("Pause", true, None);
            let resume = MenuItem::new("Resume", false, None);
            let reset = MenuItem::new("Reset", true, None);
            let quit = MenuItem::new("Quit", true, None);
            let create_slot = MenuItem::new("Create New Slot", true, None);
            let save_active = MenuItem::new("Save Active Slot", true, None);
            let load_active = MenuItem::new("Load Active Slot", false, None);

            let open_id = open.id().clone();
            let close_id = close.id().clone();
            let settings_id = settings.id().clone();
            let debugger_id = debugger.id().clone();
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
            file_menu.append(&close).unwrap();
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
            emulation_menu.append(&debugger).unwrap();

            menu_bar.append(&file_menu).unwrap();
            menu_bar.append(&emulation_menu).unwrap();

            MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
                let command = if event.id() == &open_id {
                    Some(MenuCommand::Open)
                } else if event.id() == &close_id {
                    Some(MenuCommand::Close)
                } else if event.id() == &settings_id {
                    Some(MenuCommand::Settings)
                } else if event.id() == &debugger_id {
                    Some(MenuCommand::Debugger)
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
                close,
                settings,
                debugger,
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

        pub(crate) fn update(&mut self, sync: MenuSync<'_>) {
            self.update_labels(sync.language, sync.shortcuts);
            self.open.set_enabled(!sync.settings_open);
            self.close.set_enabled(!sync.settings_open && sync.loaded);
            self.settings.set_enabled(!sync.settings_open);
            self.pause
                .set_enabled(!sync.settings_open && sync.loaded && !sync.paused);
            self.resume
                .set_enabled(!sync.settings_open && sync.loaded && sync.paused);
            self.reset.set_enabled(!sync.settings_open && sync.loaded);
            self.debugger
                .set_enabled(!sync.settings_open && sync.loaded);
            self.create_slot
                .set_enabled(!sync.settings_open && sync.loaded);
            self.save_active
                .set_enabled(!sync.settings_open && sync.loaded);
            self.load_active
                .set_enabled(!sync.settings_open && sync.loaded && sync.active_slot.is_some());
            self.rebuild_dynamic_slot_menus(sync.slots, sync.active_slot, sync.language);
        }

        pub(crate) fn clear_event_handler(&self) {
            MenuEvent::set_event_handler::<fn(MenuEvent)>(None);
        }

        fn rebuild_dynamic_slot_menus(
            &mut self,
            slots: &[StateSlotSummary],
            active_slot: Option<u64>,
            language: AppLanguage,
        ) {
            clear_submenu(&self.active_slot_menu);
            clear_submenu(&self.save_slot_menu);
            clear_submenu(&self.load_slot_menu);
            clear_submenu(&self.delete_slot_menu);
            let mut commands = Vec::new();
            if slots.is_empty() {
                let placeholder = text(language, UiText::NoSaveSlots);
                for menu in [
                    &self.active_slot_menu,
                    &self.save_slot_menu,
                    &self.load_slot_menu,
                    &self.delete_slot_menu,
                ] {
                    menu.append(&MenuItem::new(placeholder, false, None))
                        .unwrap();
                }
            }
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

        fn update_labels(&self, language: AppLanguage, shortcuts: &[ShortcutBinding]) {
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
            self.close.set_text(text(language, UiText::Close));
            self.settings
                .set_text(format!("{}...", text(language, UiText::Settings)));
            self.debugger.set_text(shortcut_hint(
                text(language, UiText::OpenDebugger),
                shortcuts,
                ShortcutAction::OpenDebugger,
            ));
            self.pause.set_text(text(language, UiText::Pause));
            self.resume.set_text(text(language, UiText::Resume));
            self.reset.set_text(text(language, UiText::Reset));
            self.quit.set_text(text(language, UiText::Quit));
            self.create_slot
                .set_text(text(language, UiText::CreateSaveSlot));
            self.save_active.set_text(shortcut_hint(
                text(language, UiText::SaveActiveSlot),
                shortcuts,
                ShortcutAction::SaveActiveSlot,
            ));
            self.load_active.set_text(shortcut_hint(
                text(language, UiText::LoadActiveSlot),
                shortcuts,
                ShortcutAction::LoadActiveSlot,
            ));
        }
    }

    /// Menu label with the action's *current* shortcut hint
    /// (`Save Active Slot (F5)`), or the bare label when unbound.
    /// Binding-driven so Settings rebinding never leaves a stale
    /// hardcoded key in the menu.
    fn shortcut_hint(base: &str, shortcuts: &[ShortcutBinding], action: ShortcutAction) -> String {
        match shortcut_key_label(shortcuts, action) {
            Some(key) => menu_label_with_shortcut(base, key),
            None => base.to_string(),
        }
    }

    fn clear_submenu(menu: &Submenu) {
        while !menu.items().is_empty() {
            let _ = menu.remove_at(0);
        }
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
    use nerust_settings_core::model::language::AppLanguage;
    use tao::event_loop::EventLoopProxy;

    use super::{MenuSync, StateSlotSummary, TaoWindow, UserEvent};

    pub(crate) struct AppMenu;

    impl AppMenu {
        pub(crate) fn new(_proxy: EventLoopProxy<UserEvent>) -> Self {
            Self
        }

        pub(crate) fn init_for_window(&self, _window: &TaoWindow) {}

        pub(crate) fn update(&mut self, _sync: MenuSync<'_>) {}

        pub(crate) fn clear_event_handler(&self) {}
    }
}
