// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use tao::window::Window as TaoWindow;

#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub(crate) enum MenuCommand {
    Pause,
    Resume,
    Reset,
    Quit,
}

#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub(crate) enum UserEvent {
    Menu(MenuCommand),
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
mod imp {
    use super::{MenuCommand, TaoWindow, UserEvent};
    #[cfg(any(
        target_os = "linux",
        target_os = "dragonfly",
        target_os = "freebsd",
        target_os = "netbsd",
        target_os = "openbsd"
    ))]
    use gtk::prelude::WidgetExt;
    use muda::{Menu, MenuEvent, MenuItem, Submenu};
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
        pause: MenuItem,
        resume: MenuItem,
    }

    impl AppMenu {
        pub(crate) fn new(proxy: EventLoopProxy<UserEvent>) -> Self {
            let menu_bar = Menu::new();
            let file_menu = Submenu::new("File", true);
            let emulation_menu = Submenu::new("Emulation", true);

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

            let pause = MenuItem::new("Pause", true, None);
            let resume = MenuItem::new("Resume", false, None);
            let reset = MenuItem::new("Reset", true, None);
            let quit = MenuItem::new("Quit", true, None);

            let pause_id = pause.id().clone();
            let resume_id = resume.id().clone();
            let reset_id = reset.id().clone();
            let quit_id = quit.id().clone();

            file_menu.append(&quit).unwrap();
            emulation_menu.append(&pause).unwrap();
            emulation_menu.append(&resume).unwrap();
            emulation_menu.append(&reset).unwrap();

            menu_bar.append(&file_menu).unwrap();
            menu_bar.append(&emulation_menu).unwrap();

            MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
                let command = if event.id() == &pause_id {
                    Some(MenuCommand::Pause)
                } else if event.id() == &resume_id {
                    Some(MenuCommand::Resume)
                } else if event.id() == &reset_id {
                    Some(MenuCommand::Reset)
                } else if event.id() == &quit_id {
                    Some(MenuCommand::Quit)
                } else {
                    None
                };
                if let Some(command) = command {
                    let _ = proxy.send_event(UserEvent::Menu(command));
                }
            }));

            Self {
                menu_bar,
                pause,
                resume,
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

        pub(crate) fn update(&self, paused: bool) {
            self.pause.set_enabled(!paused);
            self.resume.set_enabled(paused);
        }

        pub(crate) fn clear_event_handler(&self) {
            MenuEvent::set_event_handler::<fn(MenuEvent)>(None);
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
mod imp {
    use super::{TaoWindow, UserEvent};
    use tao::event_loop::EventLoopProxy;

    pub(crate) struct AppMenu;

    impl AppMenu {
        pub(crate) fn new(_proxy: EventLoopProxy<UserEvent>) -> Self {
            Self
        }

        pub(crate) fn init_for_window(&self, _window: &TaoWindow) {}

        pub(crate) fn update(&self, _paused: bool) {}

        pub(crate) fn clear_event_handler(&self) {}
    }
}

pub(crate) use imp::AppMenu;
