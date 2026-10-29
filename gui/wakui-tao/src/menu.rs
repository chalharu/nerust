use tao::window::Window as TaoWindow;
use wakui_core::menu::{MenuCapabilities, Menubar};

pub(crate) const TAO_MENU_CAPABILITIES: MenuCapabilities = MenuCapabilities {
    about: false,
    dialog_ellipsis: true,
    application_menu: cfg!(target_os = "macos"),
};

/// Headless-testable menu surface: production `AppMenu` drives the
/// native menubar, test recorders observe the syncs. Object-safe so
/// `HostState` holds it behind `Box<dyn MenuSink>`.
pub(crate) trait MenuSink {
    fn update(&mut self, loaded: bool, paused: bool, settings_open: bool, menu: &Menubar);
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
    use wakui_core::event::EventSink;
    use wakui_core::{
        commands::{CommandDescriptor, CommandVisitor},
        menu::{
            MenuEntry, MenuId, MenuItem as HostItem, MenuVisitor, Menubar, NativeApplicationEntry,
            NativeApplicationRole, Submenu as HostMenu,
        },
        menu_presenter::MenuPresenter,
    };

    use super::TaoWindow;
    use crate::event::{MenuCommand, UserEvent};

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
        application_menu: Option<(Submenu, Vec<NativeApplicationEntry>)>,
    }

    impl AppMenu {
        pub(crate) fn new(sink: Arc<dyn EventSink + Send + Sync>) -> Self {
            let menu_bar = Menu::new();

            let commands = Arc::new(RwLock::new(Vec::<(muda::MenuId, MenuCommand)>::new()));
            let handler = commands.clone();
            MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
                // Release the routing lock before dispatching into the loop.
                let command = handler
                    .read()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .iter()
                    .find_map(|(id, command)| (event.id() == id).then_some(*command));
                if let Some(command) = command
                    && !sink.send(UserEvent::Menu(command))
                {
                    log::debug!("menu event dropped after loop exit");
                }
            }));

            Self {
                menu_bar,
                menus: HashMap::new(),
                items: Vec::new(),
                commands,
                presenter: Rc::default(),
                application_menu: None,
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

        pub(crate) fn update(&mut self, menu: &Menubar) {
            let presenter = self.presenter.clone();
            let mut next = presenter.submit(menu.clone());
            while let Some(plan) = next {
                plan.visit(&mut NativeVisitor {
                    native: self,
                    stack: Vec::new(),
                    root_rebuild: false,
                    declarations: Vec::new(),
                    visited_menus: Vec::new(),
                });
                next = presenter.complete(plan);
            }
        }

        pub(crate) fn clear_event_handler(&self) {
            MenuEvent::set_event_handler::<fn(MenuEvent)>(None);
        }
    }

    fn predefined(entry: NativeApplicationEntry) -> muda::PredefinedMenuItem {
        use NativeApplicationEntry::{Role, Separator};
        use NativeApplicationRole as R;
        use muda::PredefinedMenuItem as P;
        match entry {
            Separator => P::separator(),
            Role(R::About) => P::about(None, None),
            Role(R::Services) => P::services(None),
            Role(R::Hide) => P::hide(None),
            Role(R::HideOthers) => P::hide_others(None),
            Role(R::ShowAll) => P::show_all(None),
            Role(R::Quit) => P::quit(None),
        }
    }

    struct NativeVisitor<'a> {
        native: &'a mut AppMenu,
        stack: Vec<(MenuId, bool)>,
        root_rebuild: bool,
        declarations: Vec<CommandDescriptor>,
        visited_menus: Vec<MenuId>,
    }

    impl CommandVisitor for NativeVisitor<'_> {
        fn command(&mut self, command: &CommandDescriptor) {
            // Muda routes native IDs directly, not a named action registry.
            // Declarations still validate each binding and supply enablement.
            self.declarations.push(*command);
        }
    }

    impl MenuVisitor for NativeVisitor<'_> {
        fn begin_menubar(&mut self, menubar: &Menubar, rebuild: bool) {
            self.root_rebuild = rebuild;
            if rebuild {
                for _ in 0..self.native.menu_bar.items().len() {
                    let _ = self.native.menu_bar.remove_at(0);
                }
            }
            if let Some(app) = &menubar.application_menu {
                let (native, entries) = self
                    .native
                    .application_menu
                    .get_or_insert_with(|| (Submenu::new(&app.label, true), Vec::new()));
                if native.text() != app.label {
                    native.set_text(&app.label);
                }
                if *entries != app.entries {
                    clear_submenu(native);
                    for entry in &app.entries {
                        try_append(|| native.append(&predefined(*entry)));
                    }
                    *entries = app.entries.clone();
                }
                if rebuild {
                    try_append(|| self.native.menu_bar.append(native));
                }
            } else {
                self.native.application_menu = None;
            }
        }

        fn begin_menu(&mut self, menu: &HostMenu, rebuild: bool) {
            self.visited_menus.push(menu.id);
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
            let declaration = item.command.and_then(|command| {
                let (id, argument) = command.binding();
                self.declarations
                    .iter()
                    .find(|d| d.id == id)
                    .filter(|d| d.bind(argument) == Some(command))
            });
            let enabled = item.enabled && declaration.is_some_and(|d| d.enabled);
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
            native.set_enabled(enabled);
            if rebuild {
                try_append(|| self.native.menus[&parent].append(&native));
            }
        }

        fn end_menu(&mut self, id: MenuId) {
            let ended = self.stack.pop().map(|(id, _)| id);
            debug_assert_eq!(ended, Some(id));
        }

        fn end_menubar(&mut self) {
            self.native
                .menus
                .retain(|id, _| self.visited_menus.contains(id));
            self.native
                .items
                .retain(|(id, _, _)| self.visited_menus.contains(id));
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
        fn update(&mut self, _loaded: bool, _paused: bool, _settings_open: bool, menu: &Menubar) {
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
        use super::{MenuCommand, try_append};
        use wakui_core::commands::MenuCommand as EventCommand;
        use wakui_core::commands::NativeRole;

        #[test]
        fn native_commands_are_host_commands_without_conversion() {
            let command: crate::event::MenuCommand = MenuCommand::Native(NativeRole::Quit);
            assert_eq!(command, EventCommand::Native(NativeRole::Quit));
            let about: crate::event::MenuCommand = MenuCommand::Native(NativeRole::About);
            assert_eq!(about, EventCommand::Native(NativeRole::About));
        }

        #[test]
        fn try_append_tolerates_failure() {
            // A failing append logs and continues instead of panicking.
            try_append(|| Err(muda::Error::NotAChildOfThisMenu));
            try_append(|| Ok(()));
        }

        /// Run alone under Xvfb: GTK3 owns the initializing test thread.
        /// CI-only (ignored by default): needs a display, enabled in
        /// CI with `--run-ignored only` under `xvfb-run`.
        #[cfg(target_os = "linux")]
        #[ignore = "needs display; CI-only under xvfb"]
        #[test]
        fn gtk3_native_visitor_preserves_identity_and_current_routing() {
            use std::{
                collections::HashSet,
                sync::{Arc, Mutex},
            };

            use gtk::prelude::*;
            use muda::{MenuItem, MenuItemKind};
            use wakui_core::commands::SessionCommand as S;
            use wakui_core::commands::{CommandDescriptor, CommandId, CommandParameter};
            use wakui_core::event::{EventCommand as HostEvent, EventSink};
            use wakui_core::menu::MenuCommand;
            use wakui_core::menu::{CommandModel, MenuEntry, MenuId, Menubar, Submenu};

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

            fn slot_label(id: u64, active: Option<u64>) -> String {
                if active == Some(id) {
                    format!("Slot {id} (active)")
                } else {
                    format!("Slot {id}")
                }
            }

            /// Canned model with the production shape: File/Emulation roots,
            /// States nesting the four slot submenus, labels pre-localized.
            /// Native identity, routing, and enablement are what this test
            /// owns; label copy lives view-side.
            #[allow(clippy::too_many_arguments)]
            fn canned_bar(
                loaded: bool,
                paused: bool,
                settings_open: bool,
                slots: &[u64],
                active: Option<u64>,
            ) -> Menubar {
                use wakui_core::commands::MenuCommand as C;
                let mut definitions = vec![];
                for id in [
                    CommandId::Open,
                    CommandId::Close,
                    CommandId::Settings,
                    CommandId::Debugger,
                    CommandId::NotificationHistory,
                    CommandId::Quit,
                    CommandId::Pause,
                    CommandId::Resume,
                    CommandId::Reset,
                    CommandId::CreateSlot,
                    CommandId::SaveActiveSlotOrNew,
                    CommandId::LoadActiveSlot,
                ] {
                    definitions.push(CommandDescriptor {
                        id,
                        parameter: CommandParameter::None,
                        enabled: true,
                    });
                }
                for id in [
                    CommandId::SelectActiveSlot,
                    CommandId::SaveSlot,
                    CommandId::LoadSlot,
                    CommandId::DeleteSlot,
                ] {
                    definitions.push(CommandDescriptor {
                        id,
                        parameter: CommandParameter::SlotId,
                        enabled: true,
                    });
                }
                let item_enabled = |command: Option<MenuCommand>| -> bool {
                    match command {
                        None => false,
                        Some(C::Native(wakui_core::commands::NativeRole::Quit)) => true,
                        Some(_) if settings_open => false,
                        Some(C::Open | C::Settings | C::Debugger | C::NotificationHistory) => true,
                        Some(C::Session(S::Pause)) => loaded && !paused,
                        Some(C::Session(S::Resume)) => loaded && paused,
                        Some(C::Session(S::LoadActiveSlot)) => loaded && active.is_some(),
                        Some(_) => loaded,
                    }
                };
                let leaf = |command: Option<MenuCommand>, label: &str| {
                    MenuEntry::Item(wakui_core::menu::MenuItem {
                        command,
                        label: label.to_string(),
                        enabled: item_enabled(command),
                    })
                };
                let slot_submenu =
                    |id: MenuId, label: &str, make_command: fn(u64) -> S| -> MenuEntry {
                        let children: Vec<MenuEntry> = if slots.is_empty() {
                            vec![MenuEntry::Item(wakui_core::menu::MenuItem {
                                command: None,
                                label: "No save slots".to_string(),
                                enabled: false,
                            })]
                        } else {
                            slots
                                .iter()
                                .map(|slot| {
                                    let command = C::Session(make_command(*slot));
                                    MenuEntry::Item(wakui_core::menu::MenuItem {
                                        command: Some(command),
                                        label: slot_label(*slot, active),
                                        enabled: item_enabled(Some(command)),
                                    })
                                })
                                .collect::<Vec<_>>()
                        };
                        MenuEntry::Submenu(Submenu {
                            id,
                            label: label.to_string(),
                            enabled: loaded && !settings_open && !slots.is_empty(),
                            children,
                        })
                    };
                Menubar {
                    commands: CommandModel { definitions },
                    menus: vec![
                        Submenu {
                            id: MenuId::File,
                            label: "File".to_string(),
                            enabled: true,
                            children: vec![
                                leaf(Some(C::Open), "Open..."),
                                leaf(Some(C::Close), "Close"),
                                leaf(Some(C::Settings), "Settings..."),
                                leaf(Some(C::Debugger), "Open Debugger"),
                                leaf(Some(C::NotificationHistory), "Notification History"),
                                leaf(
                                    Some(C::Native(wakui_core::commands::NativeRole::Quit)),
                                    "Quit",
                                ),
                            ],
                        },
                        Submenu {
                            id: MenuId::Emulation,
                            label: "Emulation".to_string(),
                            enabled: true,
                            children: vec![
                                leaf(Some(C::Session(S::Pause)), "Pause"),
                                leaf(Some(C::Session(S::Resume)), "Resume"),
                                leaf(Some(C::Session(S::Reset)), "Reset"),
                                leaf(Some(C::Session(S::CreateSlot)), "New Slot"),
                                leaf(Some(C::Session(S::SaveActiveSlotOrNew)), "Save"),
                                leaf(Some(C::Session(S::LoadActiveSlot)), "Load"),
                                MenuEntry::Submenu(Submenu {
                                    id: MenuId::States,
                                    label: "States".to_string(),
                                    enabled: loaded && !settings_open,
                                    children: vec![
                                        slot_submenu(
                                            MenuId::SelectSlot,
                                            "Select Slot",
                                            S::SelectActiveSlot,
                                        ),
                                        slot_submenu(MenuId::SaveSlot, "Save Slot", S::SaveSlot),
                                        slot_submenu(MenuId::LoadSlot, "Load Slot", S::LoadSlot),
                                        slot_submenu(
                                            MenuId::DeleteSlot,
                                            "Delete Slot",
                                            S::DeleteSlot,
                                        ),
                                    ],
                                }),
                            ],
                        },
                    ],
                    application_menu: None,
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

            fn sync_and_check(
                menu: &mut AppMenu,
                loaded: bool,
                paused: bool,
                settings_open: bool,
                slots: &[u64],
                active: Option<u64>,
                bar: &gtk::MenuBar,
            ) {
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
                    let menubar = canned_bar(loaded, paused, settings_open, slots, active);
                    menu.update(&menubar);
                    let roots = menu.menu_bar.items();
                    assert_eq!(roots.len(), 2);
                    for (root, id) in roots.iter().zip([MenuId::File, MenuId::Emulation]) {
                        assert_eq!(root.id(), menu.menus[&id].id());
                    }
                    assert_eq!(menu.menus.len(), 7);
                    for (id, label) in [
                        (MenuId::File, "File"),
                        (MenuId::Emulation, "Emulation"),
                        (MenuId::States, "States"),
                        (MenuId::SelectSlot, "Select Slot"),
                        (MenuId::SaveSlot, "Save Slot"),
                        (MenuId::LoadSlot, "Load Slot"),
                        (MenuId::DeleteSlot, "Delete Slot"),
                    ] {
                        assert_eq!(menu.menus[&id].text(), label);
                    }
                    for (id, count) in [
                        (MenuId::File, 6),
                        (MenuId::Emulation, 7),
                        (MenuId::States, 4),
                    ] {
                        assert_eq!(menu.menus[&id].items().len(), count);
                    }
                    let mut expected = vec![
                        (C::Open, EventCommand::Open),
                        (C::Close, EventCommand::Close),
                        (C::Settings, EventCommand::Settings),
                        (
                            C::Native(NativeRole::Quit),
                            EventCommand::Native(NativeRole::Quit),
                        ),
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
                            loaded && !settings_open && !slots.is_empty()
                        );
                        let children = submenu.items();
                        assert_eq!(children.len(), slots.len().max(1));
                        if slots.is_empty() {
                            let placeholder = children[0].as_menuitem().unwrap();
                            assert_eq!(placeholder.text(), "No save slots");
                            assert!(!placeholder.is_enabled());
                            assert!(menu.items.iter().any(|(parent, command, item)| {
                                *parent == id && command.is_none() && item.id() == placeholder.id()
                            }));
                        } else {
                            for (child, slot) in children.iter().zip(slots) {
                                let command = make_command(*slot);
                                let native = child.as_menuitem().unwrap();
                                assert_eq!(native.id(), item(menu, C::Session(command)).id());
                                assert_eq!(native.text(), slot_label(*slot, active));
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
                            Some(_) if settings_open => false,
                            Some(C::Open | C::Settings | C::Debugger | C::NotificationHistory) => {
                                true
                            }
                            Some(C::Session(S::Pause)) => loaded && !paused,
                            Some(C::Session(S::Resume)) => loaded && paused,
                            Some(C::Session(S::LoadActiveSlot)) => loaded && active.is_some(),
                            Some(_) => loaded,
                        };
                        assert_eq!(leaf.is_enabled(), enabled, "{command:?}");
                    }
                    for (command, event) in expected {
                        let native = item(menu, command);
                        let routed = routes.iter().find(|(id, _)| id == native.id()).unwrap().1;
                        assert_eq!(routed, command);
                        let transported: crate::event::MenuCommand = routed;
                        assert_eq!(transported, event);
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

            gtk::init().expect("native menu test requires Xvfb/display");
            #[derive(Default)]
            struct RecordingSink(Mutex<Vec<HostEvent>>);
            impl EventSink for RecordingSink {
                fn send(&self, event: HostEvent) -> bool {
                    self.0.lock().unwrap().push(event);
                    true
                }
            }
            let recording = Arc::new(RecordingSink::default());
            let transport: Arc<dyn EventSink + Send + Sync> = recording.clone();
            let mut menu = AppMenu::new(transport);
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
            let (mut loaded, mut paused, mut settings_open) = (false, false, false);
            let mut slots: Vec<u64> = Vec::new();
            let mut active_slot: Option<u64> = None;
            sync_and_check(
                &mut menu,
                loaded,
                paused,
                settings_open,
                &slots,
                active_slot,
                &bar,
            );
            // Activate the actual GTK leaf: muda's callback must transport the
            // canonical Host command through the production dyn sink path.
            let file = bar.children()[0]
                .clone()
                .downcast::<gtk::MenuItem>()
                .unwrap();
            let file_menu = file.submenu().unwrap().downcast::<gtk::Menu>().unwrap();
            let old_open = file_menu.children()[0]
                .clone()
                .downcast::<gtk::MenuItem>()
                .unwrap();
            old_open.activate();
            assert!(matches!(
                recording.0.lock().unwrap().as_slice(),
                [HostEvent::Menu(MenuCommand::Open)]
            ));
            assert_eq!(menu.menus[&MenuId::File].text(), "File");
            assert_eq!(item(&menu, MenuCommand::Debugger).text(), "Open Debugger");
            loaded = true;
            slots = vec![42, 7];
            active_slot = Some(42);
            sync_and_check(
                &mut menu,
                loaded,
                paused,
                settings_open,
                &slots,
                active_slot,
                &bar,
            );
            paused = true;
            sync_and_check(
                &mut menu,
                loaded,
                paused,
                settings_open,
                &slots,
                active_slot,
                &bar,
            );
            settings_open = true;
            sync_and_check(
                &mut menu,
                loaded,
                paused,
                settings_open,
                &slots,
                active_slot,
                &bar,
            );
            settings_open = false;
            active_slot = Some(7);
            sync_and_check(
                &mut menu,
                loaded,
                paused,
                settings_open,
                &slots,
                active_slot,
                &bar,
            );
            // Delete one slot, then the last: removed IDs/commands must vanish,
            // while the surviving slot and static items retain their IDs.
            slots = vec![7];
            sync_and_check(
                &mut menu,
                loaded,
                paused,
                settings_open,
                &slots,
                active_slot,
                &bar,
            );
            slots = Vec::new();
            active_slot = None;
            sync_and_check(
                &mut menu,
                loaded,
                paused,
                settings_open,
                &slots,
                active_slot,
                &bar,
            );
            loaded = false;
            sync_and_check(
                &mut menu,
                loaded,
                paused,
                settings_open,
                &slots,
                active_slot,
                &bar,
            );

            // Root structure comes from Host, even when exercising a standard
            // application menu on this test platform rather than macOS.
            use wakui_core::menu::{
                NativeApplicationEntry as A, NativeApplicationMenu, NativeApplicationRole as R,
            };
            let mut root = canned_bar(loaded, paused, settings_open, &slots, active_slot);
            root.application_menu = Some(NativeApplicationMenu {
                label: "Host App".into(),
                entries: vec![A::Role(R::About), A::Separator, A::Role(R::Quit)],
            });
            menu.update(&root.clone());
            let app = menu.application_menu.as_ref().unwrap().0.clone();
            let app_id = app.id().clone();
            let native_ids = app
                .items()
                .iter()
                .map(|item| item.id().clone())
                .collect::<Vec<_>>();
            assert_eq!(native_ids.len(), 3);
            assert_eq!(menu.menu_bar.items()[0].id(), &app_id);
            assert_eq!(
                menu.menu_bar.items()[1].id(),
                menu.menus[&MenuId::File].id()
            );
            root.application_menu.as_mut().unwrap().label = "Relabeled App".into();
            menu.update(&root.clone());
            let app = &menu.application_menu.as_ref().unwrap().0;
            assert_eq!(app.id(), &app_id);
            assert_eq!(app.text(), "Relabeled App");
            assert_eq!(
                app.items()
                    .iter()
                    .map(|item| item.id().clone())
                    .collect::<Vec<_>>(),
                native_ids
            );

            // Removing roots retires every command route, including callbacks
            // from previously retained native widgets.
            root.application_menu = None;
            root.menus.clear();
            menu.update(&root);
            assert!(menu.menu_bar.items().is_empty());
            assert!(menu.application_menu.is_none());
            assert!(menu.menus.is_empty());
            assert!(menu.items.is_empty());
            assert!(menu.commands.read().unwrap().is_empty());
            let delivered = recording.0.lock().unwrap().len();
            old_open.activate();
            assert_eq!(recording.0.lock().unwrap().len(), delivered);
            menu.menu_bar.remove_for_gtk_window(&window).unwrap();
            menu.clear_event_handler();
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
    use super::TaoWindow;
    use std::sync::Arc;
    use wakui_core::event::EventSink;
    use wakui_core::menu::Menubar;

    pub(crate) struct AppMenu;

    impl AppMenu {
        pub(crate) fn new(_sink: Arc<dyn EventSink + Send + Sync>) -> Self {
            Self
        }

        pub(crate) fn init_for_window(&self, _window: &TaoWindow) {}

        pub(crate) fn update(
            &mut self,
            _loaded: bool,
            _paused: bool,
            _settings_open: bool,
            _menu: &Menubar,
        ) {
        }

        pub(crate) fn clear_event_handler(&self) {}
    }

    impl super::MenuSink for AppMenu {
        fn update(&mut self, _loaded: bool, _paused: bool, _settings_open: bool, menu: &Menubar) {
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
