use nerust_frontend_host::application::MenuSig;
use tao::window::Window as TaoWindow;

/// Headless-testable menu surface: production `AppMenu` drives the
/// native menubar, test recorders observe the syncs. Object-safe so
/// `HostState` holds it behind `Box<dyn MenuSink>`.
pub(crate) trait MenuSink {
    fn update(&mut self, menu: MenuSig);
    fn init_for_window(&self, window: &TaoWindow);
    fn clear_event_handler(&self);
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
    use std::{
        collections::HashMap,
        rc::Rc,
        sync::{Arc, RwLock},
    };

    #[cfg(any(
        target_os = "linux",
        target_os = "dragonfly",
        target_os = "freebsd",
        target_os = "netbsd",
        target_os = "openbsd"
    ))]
    use gtk::prelude::WidgetExt;
    use muda::{Menu, MenuEvent, MenuItem, Submenu};
    use nerust_frontend_host::{
        application::MenuSig,
        menu::{
            MenuCapabilities, MenuCommand, MenuEntry, MenuId, MenuItem as HostItem, MenuModel,
            MenuVisitor, NativeRole, Submenu as HostMenu,
        },
        widgets::MenuPresenter,
    };
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

    use super::TaoWindow;
    use crate::event::MenuCommand as EventCommand;
    use crate::event::UserEvent;

    /// Best-effort menu construction: a failed append (torn-down
    /// native menu) logs and leaves a partial menu instead of
    /// aborting the whole frontend at startup.
    fn try_append(append: impl FnOnce() -> muda::Result<()>) {
        if let Err(error) = append() {
            log::error!("failed to add menu item: {error}");
        }
    }

    pub(crate) struct AppMenu {
        menu_bar: Menu,
        menus: HashMap<MenuId, Submenu>,
        items: Vec<(MenuId, Option<MenuCommand>, MenuItem)>,
        commands: Arc<RwLock<Vec<(muda::MenuId, MenuCommand)>>>,
        presenter: Rc<MenuPresenter>,
    }

    impl AppMenu {
        pub(crate) fn new(proxy: EventLoopProxy<UserEvent>) -> Self {
            let menu_bar = Menu::new();

            #[cfg(target_os = "macos")]
            {
                let app_menu = Submenu::new("App", true);
                try_append(|| {
                    app_menu.append_items(&[
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
                });
                try_append(|| menu_bar.append(&app_menu));
            }

            let commands = Arc::new(RwLock::new(Vec::<(muda::MenuId, MenuCommand)>::new()));
            let handler = commands.clone();
            MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
                // Release the routing lock before dispatching into the loop.
                let command = handler
                    .read()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .iter()
                    .find_map(|(id, command)| (event.id() == id).then_some(*command));
                if let Some(command) = command.and_then(event_command)
                    && let Err(error) = proxy.send_event(UserEvent::Menu(command))
                {
                    log::debug!("menu event dropped after loop exit: {error}");
                }
            }));

            Self {
                menu_bar,
                menus: HashMap::new(),
                items: Vec::new(),
                commands,
                presenter: Rc::default(),
            }
        }

        pub(crate) fn init_for_window(&self, window: &TaoWindow) {
            #[cfg(target_os = "windows")]
            unsafe {
                if let Err(error) = self.menu_bar.init_for_hwnd(window.hwnd() as _) {
                    log::error!("failed to attach menu to window: {error}");
                }
            }

            #[cfg(any(
                target_os = "linux",
                target_os = "dragonfly",
                target_os = "freebsd",
                target_os = "netbsd",
                target_os = "openbsd"
            ))]
            {
                if let Err(error) = self
                    .menu_bar
                    .init_for_gtk_window(window.gtk_window(), window.default_vbox())
                {
                    log::error!("failed to attach menu to window: {error}");
                }
                window.gtk_window().show_all();
            }

            #[cfg(target_os = "macos")]
            {
                let _ = window.ns_view();
                self.menu_bar.init_for_nsapp();
            }
        }

        pub(crate) fn update(&mut self, menu: MenuSig) {
            let presenter = self.presenter.clone();
            let mut next = presenter.submit(MenuModel::build(
                &menu,
                MenuCapabilities {
                    about: false,
                    dialog_ellipsis: true,
                },
            ));
            while let Some(plan) = next {
                plan.visit(&mut NativeVisitor {
                    native: self,
                    stack: Vec::new(),
                    root_rebuild: false,
                });
                next = presenter.complete(plan);
            }
        }

        pub(crate) fn clear_event_handler(&self) {
            MenuEvent::set_event_handler::<fn(MenuEvent)>(None);
        }
    }

    fn event_command(command: MenuCommand) -> Option<EventCommand> {
        Some(match command {
            MenuCommand::Open => EventCommand::Open,
            MenuCommand::Close => EventCommand::Close,
            MenuCommand::Settings => EventCommand::Settings,
            MenuCommand::Debugger => EventCommand::Debugger,
            MenuCommand::NotificationHistory => EventCommand::NotificationHistory,
            MenuCommand::Session(command) => EventCommand::Session(command),
            MenuCommand::Native(NativeRole::Quit) => EventCommand::Quit,
            MenuCommand::Native(NativeRole::About) => return None,
        })
    }

    struct NativeVisitor<'a> {
        native: &'a mut AppMenu,
        stack: Vec<(MenuId, bool)>,
        root_rebuild: bool,
    }

    impl MenuVisitor for NativeVisitor<'_> {
        fn begin_model(&mut self, rebuild: bool) {
            self.root_rebuild = rebuild;
            if rebuild {
                // Keep the native standard App submenu on macOS.
                let start = usize::from(cfg!(target_os = "macos"));
                for _ in start..self.native.menu_bar.items().len() {
                    let _ = self.native.menu_bar.remove_at(start);
                }
            }
        }

        fn begin_menu(&mut self, menu: &HostMenu, rebuild: bool) {
            let native = self
                .native
                .menus
                .entry(menu.id)
                .or_insert_with(|| Submenu::new(&menu.label, menu.enabled))
                .clone();
            if native.text() != menu.label {
                native.set_text(&menu.label);
            }
            native.set_enabled(menu.enabled);
            if let Some((parent, parent_rebuild)) = self.stack.last() {
                if *parent_rebuild {
                    try_append(|| self.native.menus[parent].append(&native));
                }
            } else if self.root_rebuild {
                try_append(|| self.native.menu_bar.append(&native));
            }
            if rebuild {
                clear_submenu(&native);
                self.native.items.retain(|(parent, command, _)| {
                    *parent != menu.id || menu.children.iter().any(|entry| {
                        matches!(entry, MenuEntry::Item(item) if item.command == *command)
                    })
                });
            }
            self.stack.push((menu.id, rebuild));
        }

        fn item(&mut self, item: &HostItem) {
            let (parent, rebuild) = *self.stack.last().expect("host item parent");
            let native = if let Some((_, _, native)) = self
                .native
                .items
                .iter()
                .find(|(id, command, _)| *id == parent && *command == item.command)
            {
                native.clone()
            } else {
                let native = MenuItem::new(&item.label, item.enabled, None);
                self.native
                    .items
                    .push((parent, item.command, native.clone()));
                native
            };
            if native.text() != item.label {
                native.set_text(&item.label);
            }
            native.set_enabled(item.enabled);
            if rebuild {
                try_append(|| self.native.menus[&parent].append(&native));
            }
        }

        fn end_menu(&mut self, id: MenuId) {
            let ended = self.stack.pop().map(|(id, _)| id);
            debug_assert_eq!(ended, Some(id));
        }

        fn end_model(&mut self) {
            *self
                .native
                .commands
                .write()
                .unwrap_or_else(std::sync::PoisonError::into_inner) = self
                .native
                .items
                .iter()
                .filter_map(|(_, command, item)| {
                    command.map(|command| (item.id().clone(), command))
                })
                .collect();
        }
    }

    impl super::MenuSink for AppMenu {
        fn update(&mut self, menu: MenuSig) {
            AppMenu::update(self, menu);
        }

        fn init_for_window(&self, window: &super::TaoWindow) {
            AppMenu::init_for_window(self, window);
        }

        fn clear_event_handler(&self) {
            AppMenu::clear_event_handler(self);
        }
    }

    fn clear_submenu(menu: &Submenu) {
        // Bounded: remove_at can fail (leaving the item in place),
        // so a while-is-not-empty loop could spin forever.
        for _ in 0..menu.items().len() {
            let _ = menu.remove_at(0);
        }
    }

    #[cfg(test)]
    mod tests {
        use super::{EventCommand, MenuCommand, NativeRole, event_command, try_append};

        #[test]
        fn native_roles_route_only_supported_commands() {
            assert_eq!(
                event_command(MenuCommand::Native(NativeRole::Quit)),
                Some(EventCommand::Quit)
            );
            assert_eq!(event_command(MenuCommand::Native(NativeRole::About)), None);
        }

        #[test]
        fn try_append_tolerates_failure() {
            // A failing append logs and continues instead of panicking.
            try_append(|| Err(muda::Error::NotAChildOfThisMenu));
            try_append(|| Ok(()));
        }

        /// Run alone under Xvfb: GTK3 owns the initializing test thread.
        #[cfg(target_os = "linux")]
        #[test]
        fn gtk3_native_visitor_preserves_identity_and_current_routing() {
            use std::{
                collections::HashSet,
                rc::Rc,
                sync::{Arc, RwLock},
            };

            use gtk::prelude::*;
            use muda::{Menu, MenuItem, MenuItemKind};
            use nerust_frontend_host::{application::MenuSig, menu::MenuId};
            use nerust_gui_runtime::slots::slot_label;
            use nerust_gui_shell::session::commands::SessionCommand as S;
            use nerust_keyboard::Key;
            use nerust_persistence::{model::StateSlotSummary, sidecar::StorageLocation};
            use nerust_settings_core::{
                i18n::{UiText, text},
                model::{
                    input::{ShortcutAction, ShortcutBinding, default_shortcut_bindings},
                    language::AppLanguage,
                },
            };

            use super::AppMenu;

            // Submenus can contain a custom Box/AccelLabel rather than a label
            // property. Inspect the actual GTK widget, not just muda's state.
            fn gtk_label(widget: &gtk::Widget) -> Option<String> {
                if let Some(label) = widget.downcast_ref::<gtk::Label>() {
                    return Some(label.text().to_string());
                }
                widget
                    .downcast_ref::<gtk::Container>()?
                    .children()
                    .iter()
                    .find_map(gtk_label)
            }

            fn walk(
                native: Vec<MenuItemKind>,
                widgets: Vec<gtk::Widget>,
                seen: &mut HashSet<muda::MenuId>,
            ) -> Vec<MenuItem> {
                assert_eq!(native.len(), widgets.len(), "GTK children diverged");
                let mut leaves = Vec::new();
                for (entry, widget) in native.into_iter().zip(widgets) {
                    assert!(seen.insert(entry.id().clone()), "duplicate native ID");
                    let gtk_item = widget.downcast::<gtk::MenuItem>().unwrap();
                    let (label, enabled) = match entry {
                        MenuItemKind::Submenu(menu) => {
                            let children = gtk_item
                                .submenu()
                                .unwrap()
                                .downcast::<gtk::Menu>()
                                .unwrap()
                                .children();
                            leaves.extend(walk(menu.items(), children, seen));
                            (menu.text(), menu.is_enabled())
                        }
                        MenuItemKind::MenuItem(item) => {
                            assert!(gtk_item.submenu().is_none());
                            let state = (item.text(), item.is_enabled());
                            leaves.push(item);
                            state
                        }
                        _ => panic!("unexpected native menu kind"),
                    };
                    assert_eq!(
                        gtk_label(gtk_item.upcast_ref()).as_deref(),
                        Some(label.as_str())
                    );
                    assert_eq!(gtk_item.is_sensitive(), enabled);
                }
                leaves
            }

            fn slot(id: u64) -> StateSlotSummary {
                StateSlotSummary {
                    schema_version: 1,
                    slot_id: id,
                    path: StorageLocation::Native(format!("slot-{id}.nst").into()),
                    saved_at: std::time::UNIX_EPOCH,
                    has_thumbnail: false,
                    emulator_version: "native-menu-test".into(),
                }
            }

            fn item(menu: &AppMenu, command: MenuCommand) -> MenuItem {
                menu.items
                    .iter()
                    .find(|(_, value, _)| *value == Some(command))
                    .expect("command item")
                    .2
                    .clone()
            }

            fn sync_and_check(menu: &mut AppMenu, sig: &MenuSig, bar: &gtk::MenuBar) {
                use MenuCommand as C;

                // Fresh, owned allocations with equal content must not append
                // duplicates or replace identities on repeated submissions.
                for _ in 0..3 {
                    let old_items = menu
                        .items
                        .iter()
                        .map(|(parent, command, item)| (*parent, *command, item.id().clone()))
                        .collect::<Vec<_>>();
                    let old_menus = menu
                        .menus
                        .iter()
                        .map(|(id, menu)| (*id, menu.id().clone()))
                        .collect::<Vec<_>>();
                    let slots = sig.slots.to_vec();
                    let shortcuts = sig.shortcuts.to_vec();
                    menu.update(MenuSig {
                        loaded: sig.loaded,
                        paused: sig.paused,
                        slots: slots.clone().into(),
                        active_slot: sig.active_slot,
                        settings_open: sig.settings_open,
                        language: sig.language,
                        shortcuts: shortcuts.into(),
                    });
                    let roots = menu.menu_bar.items();
                    assert_eq!(roots.len(), 2);
                    for (root, id) in roots.iter().zip([MenuId::File, MenuId::Emulation]) {
                        assert_eq!(root.id(), menu.menus[&id].id());
                    }
                    assert_eq!(menu.menus.len(), 7);
                    for (id, key) in [
                        (MenuId::File, UiText::File),
                        (MenuId::Emulation, UiText::Emulation),
                        (MenuId::States, UiText::SaveStates),
                        (MenuId::SelectSlot, UiText::SelectActiveSlot),
                        (MenuId::SaveSlot, UiText::SaveSlot),
                        (MenuId::LoadSlot, UiText::LoadSlot),
                        (MenuId::DeleteSlot, UiText::DeleteSlot),
                    ] {
                        assert_eq!(menu.menus[&id].text(), text(sig.language, key));
                    }
                    for (id, count) in [
                        (MenuId::File, 4),
                        (MenuId::Emulation, 6),
                        (MenuId::States, 7),
                    ] {
                        assert_eq!(menu.menus[&id].items().len(), count);
                    }
                    let mut expected = vec![
                        (C::Open, EventCommand::Open),
                        (C::Close, EventCommand::Close),
                        (C::Settings, EventCommand::Settings),
                        (C::Native(NativeRole::Quit), EventCommand::Quit),
                        (C::Debugger, EventCommand::Debugger),
                        (C::NotificationHistory, EventCommand::NotificationHistory),
                    ];
                    for command in [
                        S::Pause,
                        S::Resume,
                        S::Reset,
                        S::CreateSlot,
                        S::SaveActiveSlotOrNew,
                        S::LoadActiveSlot,
                    ] {
                        expected.push((C::Session(command), EventCommand::Session(command)));
                    }
                    for (id, make_command) in [
                        (MenuId::SelectSlot, S::SelectActiveSlot as fn(u64) -> S),
                        (MenuId::SaveSlot, S::SaveSlot),
                        (MenuId::LoadSlot, S::LoadSlot),
                        (MenuId::DeleteSlot, S::DeleteSlot),
                    ] {
                        let submenu = &menu.menus[&id];
                        assert_eq!(
                            submenu.is_enabled(),
                            sig.loaded && !sig.settings_open && !slots.is_empty()
                        );
                        let children = submenu.items();
                        assert_eq!(children.len(), slots.len().max(1));
                        if slots.is_empty() {
                            let placeholder = children[0].as_menuitem().unwrap();
                            assert_eq!(placeholder.text(), text(sig.language, UiText::NoSaveSlots));
                            assert!(!placeholder.is_enabled());
                            assert!(menu.items.iter().any(|(parent, command, item)| {
                                *parent == id && command.is_none() && item.id() == placeholder.id()
                            }));
                        } else {
                            for (child, slot) in children.iter().zip(&slots) {
                                let command = make_command(slot.slot_id);
                                let native = child.as_menuitem().unwrap();
                                assert_eq!(native.id(), item(menu, C::Session(command)).id());
                                assert_eq!(native.text(), slot_label(slot, sig.active_slot));
                                expected
                                    .push((C::Session(command), EventCommand::Session(command)));
                            }
                        }
                    }
                    let leaves = walk(roots, bar.children(), &mut HashSet::new());
                    assert_eq!(leaves.len(), 12 + 4 * slots.len().max(1));
                    assert_eq!(menu.items.len(), leaves.len(), "stale cached items");
                    let routes = menu.commands.read().unwrap();
                    assert_eq!(routes.len(), expected.len());
                    assert_eq!(
                        routes
                            .iter()
                            .map(|(id, _)| id)
                            .collect::<HashSet<_>>()
                            .len(),
                        routes.len()
                    );
                    for leaf in &leaves {
                        let cached = menu
                            .items
                            .iter()
                            .filter(|(_, _, item)| item.id() == leaf.id())
                            .collect::<Vec<_>>();
                        assert_eq!(cached.len(), 1);
                        let command = cached[0].1;
                        let routed = routes
                            .iter()
                            .find(|(id, _)| id == leaf.id())
                            .map(|(_, c)| *c);
                        assert_eq!(routed, command, "native ID must route its current command");
                        let enabled = match command {
                            None => false,
                            Some(C::Native(NativeRole::Quit)) => true,
                            Some(_) if sig.settings_open => false,
                            Some(C::Open | C::Settings | C::Debugger | C::NotificationHistory) => {
                                true
                            }
                            Some(C::Session(S::Pause)) => sig.loaded && !sig.paused,
                            Some(C::Session(S::Resume)) => sig.loaded && sig.paused,
                            Some(C::Session(S::LoadActiveSlot)) => {
                                sig.loaded && sig.active_slot.is_some()
                            }
                            Some(_) => sig.loaded,
                        };
                        assert_eq!(leaf.is_enabled(), enabled, "{command:?}");
                    }
                    for (command, event) in expected {
                        let native = item(menu, command);
                        let routed = routes.iter().find(|(id, _)| id == native.id()).unwrap().1;
                        assert_eq!(routed, command);
                        assert_eq!(event_command(routed), Some(event));
                    }
                    for (parent, command, id) in old_items {
                        if let Some((_, _, current)) = menu
                            .items
                            .iter()
                            .find(|(p, c, _)| *p == parent && *c == command)
                        {
                            assert_eq!(current.id(), &id, "unchanged command identity");
                        } else {
                            assert!(
                                !leaves.iter().any(|item| item.id() == &id),
                                "ghost native item"
                            );
                            assert!(
                                !routes.iter().any(|(current, _)| current == &id),
                                "stale routing"
                            );
                        }
                    }
                    for (id, native_id) in old_menus {
                        assert_eq!(menu.menus[&id].id(), &native_id);
                    }
                }
            }

            if let Err(error) = gtk::init() {
                eprintln!("SKIP: native menu test needs a display (run under Xvfb): {error}");
                return;
            }
            // Do not create a Tao event loop/proxy or install a global handler.
            let mut menu = AppMenu {
                menu_bar: Menu::new(),
                menus: Default::default(),
                items: Vec::new(),
                commands: Arc::new(RwLock::new(Vec::new())),
                presenter: Rc::default(),
            };
            let window = gtk::Window::new(gtk::WindowType::Toplevel);
            let container = gtk::Box::new(gtk::Orientation::Vertical, 0);
            window.add(&container);
            menu.menu_bar
                .init_for_gtk_window(&window, Some(&container))
                .unwrap();
            let bar = menu
                .menu_bar
                .clone()
                .gtk_menubar_for_gtk_window(&window)
                .unwrap();
            let mut sig = MenuSig {
                loaded: false,
                paused: false,
                settings_open: false,
                language: AppLanguage::English,
                active_slot: None,
                slots: Vec::new().into(),
                shortcuts: default_shortcut_bindings().into(),
            };
            sync_and_check(&mut menu, &sig, &bar);
            assert_eq!(menu.menus[&MenuId::File].text(), "File");
            assert_eq!(
                item(&menu, MenuCommand::Debugger).text(),
                "Open Debugger (F4)"
            );
            sig.loaded = true;
            sig.slots = vec![slot(42), slot(7)].into();
            sig.active_slot = Some(42);
            sync_and_check(&mut menu, &sig, &bar);
            sig.paused = true;
            sync_and_check(&mut menu, &sig, &bar);
            sig.settings_open = true;
            sync_and_check(&mut menu, &sig, &bar);
            sig.settings_open = false;
            sig.language = AppLanguage::Japanese;
            sig.shortcuts = vec![
                ShortcutBinding {
                    action: ShortcutAction::OpenDebugger,
                    key: Some(Key::F2),
                },
                ShortcutBinding {
                    action: ShortcutAction::SaveActiveSlot,
                    key: Some(Key::F3),
                },
                ShortcutBinding {
                    action: ShortcutAction::LoadActiveSlot,
                    key: None,
                },
            ]
            .into();
            sig.active_slot = Some(7);
            sync_and_check(&mut menu, &sig, &bar);
            assert_eq!(menu.menus[&MenuId::File].text(), "ファイル");
            assert_eq!(
                menu.menus[&MenuId::Emulation].text(),
                text(sig.language, UiText::Emulation)
            );
            assert_eq!(
                item(&menu, MenuCommand::Open).text(),
                format!("{}...", text(sig.language, UiText::Open))
            );
            assert_eq!(
                item(&menu, MenuCommand::Debugger).text(),
                format!("{} (F2)", text(sig.language, UiText::OpenDebugger))
            );
            assert_eq!(
                item(&menu, MenuCommand::Session(S::SaveActiveSlotOrNew)).text(),
                format!("{} (F3)", text(sig.language, UiText::SaveActiveSlot))
            );
            assert_eq!(
                item(&menu, MenuCommand::Session(S::LoadActiveSlot)).text(),
                text(sig.language, UiText::LoadActiveSlot)
            );
            // Delete one slot, then the last: removed IDs/commands must vanish,
            // while the surviving slot and static items retain their IDs.
            sig.slots = vec![slot(7)].into();
            sync_and_check(&mut menu, &sig, &bar);
            sig.slots = Vec::new().into();
            sig.active_slot = None;
            sync_and_check(&mut menu, &sig, &bar);
            sig.loaded = false;
            sync_and_check(&mut menu, &sig, &bar);
            menu.menu_bar.remove_for_gtk_window(&window).unwrap();
            unsafe { window.destroy() };
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
    use tao::event_loop::EventLoopProxy;

    use super::{MenuSig, TaoWindow};
    use crate::event::UserEvent;

    pub(crate) struct AppMenu;

    impl AppMenu {
        pub(crate) fn new(_proxy: EventLoopProxy<UserEvent>) -> Self {
            Self
        }

        pub(crate) fn init_for_window(&self, _window: &TaoWindow) {}

        pub(crate) fn update(&mut self, _menu: MenuSig) {}

        pub(crate) fn clear_event_handler(&self) {}
    }

    impl super::MenuSink for AppMenu {
        fn update(&mut self, menu: MenuSig) {
            AppMenu::update(self, menu);
        }

        fn init_for_window(&self, window: &super::TaoWindow) {
            AppMenu::init_for_window(self, window);
        }

        fn clear_event_handler(&self) {
            AppMenu::clear_event_handler(self);
        }
    }
}
