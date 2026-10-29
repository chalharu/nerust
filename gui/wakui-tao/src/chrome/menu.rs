use tao::window::Window as TaoWindow;
use wakui_core::chrome::menu::{MenuCapabilities, Menubar};

pub(crate) const TAO_MENU_CAPABILITIES: MenuCapabilities = MenuCapabilities {
    about: false,
    dialog_ellipsis: true,
    application_menu: cfg!(target_os = "macos"),
};

/// Headless-testable menu surface: production `AppMenu` drives the
/// native menubar, test recorders observe the syncs. Object-safe so
/// `HostState` holds it behind `Box<dyn MenuSink>`.
pub(crate) trait MenuSink {
    fn update(&mut self, loaded: bool, paused: bool, menu: &Menubar);
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
    use wakui_core::input::event::EventSink;
    use wakui_core::{
        chrome::menu::{
            MenuEntry, MenuId, MenuItem as HostItem, MenuVisitor, Menubar, NativeApplicationEntry,
            NativeApplicationRole, Submenu as HostMenu,
        },
        chrome::menu_presenter::MenuPresenter,
        input::commands::{Command, CommandRef, CommandVisitor},
    };

    use super::TaoWindow;
    use crate::event_sink::UserEvent;

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
        items: Vec<(MenuId, Option<CommandRef>, MenuItem)>,
        commands: Arc<RwLock<Vec<(muda::MenuId, CommandRef)>>>,
        presenter: Rc<MenuPresenter>,
        application_menu: Option<(Submenu, Vec<NativeApplicationEntry>)>,
    }

    impl AppMenu {
        pub(crate) fn new(sink: Arc<dyn EventSink + Send + Sync>) -> Self {
            let menu_bar = Menu::new();

            let commands = Arc::new(RwLock::new(Vec::<(muda::MenuId, CommandRef)>::new()));
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
        declarations: Vec<Command>,
        visited_menus: Vec<MenuId>,
    }

    impl CommandVisitor for NativeVisitor<'_> {
        fn command(&mut self, command: &Command) {
            // Muda routes native IDs directly, not a named action registry.
            // Declarations still validate each binding and supply enablement.
            self.declarations.push(command.clone());
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
            // Structural validation only: same opaque id plus same argument
            // shape. Domain meaning (e.g. item existence) resolved view-side.
            let declaration = item.command.and_then(|activation| {
                self.declarations
                    .iter()
                    .find(|declaration| declaration.accepts(activation))
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
        fn update(&mut self, _loaded: bool, _paused: bool, menu: &Menubar) {
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
        use super::try_append;
        use crate::testkit::{FakeCommands, TestMenu, TestSession, menu_id};
        use wakui_core::input::commands::CommandRef;
        use wakui_core::input::event::EventCommand as HostEvent;

        #[test]
        fn opaque_refs_route_without_domain_knowledge() {
            // The flavor never names domain commands: routing keys are
            // opaque ids plus an optional argument, resolved view-side.
            let commands = FakeCommands::new();
            let open = commands.reference(TestMenu::Open);
            let about = commands.reference(TestMenu::About);
            assert_ne!(open, about);
            assert_eq!(commands.resolve(open), Some(TestMenu::Open));
            let transported: HostEvent = HostEvent::Menu(open);
            assert!(matches!(transported, HostEvent::Menu(_)));
            // Wrong shapes never resolve, like malformed native bindings.
            let flipped = CommandRef {
                id: open.id,
                argument: Some(1),
            };
            assert_eq!(commands.resolve(flipped), None);
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
            use wakui_core::chrome::menu::{MenuEntry, MenuId, Menubar, Submenu};
            use wakui_core::input::event::{EventCommand as HostEvent, EventSink};

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

            fn item_label(id: u64, active: Option<u64>) -> String {
                if active == Some(id) {
                    format!("Item {id} (active)")
                } else {
                    format!("Item {id}")
                }
            }

            /// Canned model with the production shape: Group A/B roots,
            /// Group C nesting the four item submenus, labels pre-localized.
            /// Native identity, routing, and enablement are what this test
            /// owns; label copy and domain meaning live in the fake table.
            fn canned_bar(
                commands: &FakeCommands,
                loaded: bool,
                paused: bool,
                guard_open: bool,
                items: &[u64],
                active: Option<u64>,
            ) -> Menubar {
                use TestMenu as C;
                use TestSession as S;
                let reference = |command: TestMenu| commands.reference(command);
                let item_enabled = |command: Option<CommandRef>| -> bool {
                    let Some(command) = command.and_then(|activation| commands.resolve(activation))
                    else {
                        return false;
                    };
                    match command {
                        C::Quit => true,
                        _ if guard_open => false,
                        C::Open | C::AuxAlpha | C::AuxGamma | C::AuxBeta => true,
                        C::Session(S::Pause) => loaded && !paused,
                        C::Session(S::Resume) => loaded && paused,
                        C::Session(S::LoadActive) => loaded && active.is_some(),
                        _ => loaded,
                    }
                };
                let leaf = |command: Option<TestMenu>, label: &str| {
                    let activation = command.map(reference);
                    MenuEntry::Item(wakui_core::chrome::menu::MenuItem {
                        command: activation,
                        label: label.to_string(),
                        enabled: item_enabled(activation),
                    })
                };
                let item_submenu =
                    |id: MenuId, label: &str, make_command: fn(u64) -> S| -> MenuEntry {
                        let children: Vec<MenuEntry> = if items.is_empty() {
                            vec![MenuEntry::Item(wakui_core::chrome::menu::MenuItem {
                                command: None,
                                label: "No items".to_string(),
                                enabled: false,
                            })]
                        } else {
                            items
                                .iter()
                                .map(|item| {
                                    let activation = reference(C::Session(make_command(*item)));
                                    MenuEntry::Item(wakui_core::chrome::menu::MenuItem {
                                        command: Some(activation),
                                        label: item_label(*item, active),
                                        enabled: item_enabled(Some(activation)),
                                    })
                                })
                                .collect::<Vec<_>>()
                        };
                        MenuEntry::Submenu(Submenu {
                            id,
                            label: label.to_string(),
                            enabled: loaded && !guard_open && !items.is_empty(),
                            children,
                        })
                    };
                Menubar {
                    commands: commands.model(),
                    menus: vec![
                        Submenu {
                            id: menu_id::GROUP_A,
                            label: "Group A".to_string(),
                            enabled: true,
                            children: vec![
                                leaf(Some(C::Open), "Open..."),
                                leaf(Some(C::Close), "Close"),
                                leaf(Some(C::AuxAlpha), "Aux Alpha..."),
                                leaf(Some(C::AuxGamma), "Aux Gamma"),
                                leaf(Some(C::AuxBeta), "Aux Beta"),
                                leaf(Some(C::Quit), "Quit"),
                            ],
                        },
                        Submenu {
                            id: menu_id::GROUP_B,
                            label: "Group B".to_string(),
                            enabled: true,
                            children: vec![
                                leaf(Some(C::Session(S::Pause)), "Pause"),
                                leaf(Some(C::Session(S::Resume)), "Resume"),
                                leaf(Some(C::Session(S::Restart)), "Reset"),
                                leaf(Some(C::Session(S::Create)), "New item"),
                                leaf(Some(C::Session(S::SaveActiveOrNew)), "Save"),
                                leaf(Some(C::Session(S::LoadActive)), "Load"),
                                MenuEntry::Submenu(Submenu {
                                    id: menu_id::GROUP_C,
                                    label: "Group C".to_string(),
                                    enabled: loaded && !guard_open,
                                    children: vec![
                                        item_submenu(menu_id::SUB_A, "Sub A", S::Select),
                                        item_submenu(menu_id::SUB_B, "Sub B", S::Save),
                                        item_submenu(menu_id::SUB_C, "Sub C", S::Load),
                                        item_submenu(menu_id::SUB_D, "Sub D", S::Delete),
                                    ],
                                }),
                            ],
                        },
                    ],
                    application_menu: None,
                }
            }

            fn item(menu: &AppMenu, command: CommandRef) -> MenuItem {
                menu.items
                    .iter()
                    .find(|(_, value, _)| *value == Some(command))
                    .expect("command item")
                    .2
                    .clone()
            }

            #[allow(clippy::too_many_arguments)]
            fn sync_and_check(
                commands: &FakeCommands,
                menu: &mut AppMenu,
                loaded: bool,
                paused: bool,
                guard_open: bool,
                items: &[u64],
                active: Option<u64>,
                bar: &gtk::MenuBar,
            ) {
                use TestMenu as C;
                use TestSession as S;
                let reference = |command: TestMenu| commands.reference(command);

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
                    let menubar = canned_bar(commands, loaded, paused, guard_open, items, active);
                    menu.update(&menubar);
                    let roots = menu.menu_bar.items();
                    assert_eq!(roots.len(), 2);
                    for (root, id) in roots.iter().zip([menu_id::GROUP_A, menu_id::GROUP_B]) {
                        assert_eq!(root.id(), menu.menus[&id].id());
                    }
                    assert_eq!(menu.menus.len(), 7);
                    for (id, label) in [
                        (menu_id::GROUP_A, "Group A"),
                        (menu_id::GROUP_B, "Group B"),
                        (menu_id::GROUP_C, "Group C"),
                        (menu_id::SUB_A, "Sub A"),
                        (menu_id::SUB_B, "Sub B"),
                        (menu_id::SUB_C, "Sub C"),
                        (menu_id::SUB_D, "Sub D"),
                    ] {
                        assert_eq!(menu.menus[&id].text(), label);
                    }
                    for (id, count) in [
                        (menu_id::GROUP_A, 6),
                        (menu_id::GROUP_B, 7),
                        (menu_id::GROUP_C, 4),
                    ] {
                        assert_eq!(menu.menus[&id].items().len(), count);
                    }
                    let mut expected = vec![
                        reference(C::Open),
                        reference(C::Close),
                        reference(C::AuxAlpha),
                        reference(C::Quit),
                        reference(C::AuxGamma),
                        reference(C::AuxBeta),
                    ];
                    for session in [
                        S::Pause,
                        S::Resume,
                        S::Restart,
                        S::Create,
                        S::SaveActiveOrNew,
                        S::LoadActive,
                    ] {
                        expected.push(reference(C::Session(session)));
                    }
                    for (id, make_command) in [
                        (menu_id::SUB_A, S::Select as fn(u64) -> S),
                        (menu_id::SUB_B, S::Save),
                        (menu_id::SUB_C, S::Load),
                        (menu_id::SUB_D, S::Delete),
                    ] {
                        let submenu = &menu.menus[&id];
                        assert_eq!(
                            submenu.is_enabled(),
                            loaded && !guard_open && !items.is_empty()
                        );
                        let children = submenu.items();
                        assert_eq!(children.len(), items.len().max(1));
                        if items.is_empty() {
                            let placeholder = children[0].as_menuitem().unwrap();
                            assert_eq!(placeholder.text(), "No items");
                            assert!(!placeholder.is_enabled());
                            assert!(menu.items.iter().any(|(parent, command, item)| {
                                *parent == id && command.is_none() && item.id() == placeholder.id()
                            }));
                        } else {
                            for (child, id) in children.iter().zip(items) {
                                let activation = reference(C::Session(make_command(*id)));
                                let native = child.as_menuitem().unwrap();
                                assert_eq!(native.id(), item(menu, activation).id());
                                assert_eq!(native.text(), item_label(*id, active));
                                expected.push(activation);
                            }
                        }
                    }
                    let leaves = walk(roots, bar.children(), &mut HashSet::new());
                    assert_eq!(leaves.len(), 12 + 4 * items.len().max(1));
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
                        let resolved = command.and_then(|activation| commands.resolve(activation));
                        let enabled = match resolved {
                            None => false,
                            Some(TestMenu::Quit) => true,
                            _ if guard_open => false,
                            Some(
                                TestMenu::Open
                                | TestMenu::AuxAlpha
                                | TestMenu::AuxGamma
                                | TestMenu::AuxBeta,
                            ) => true,
                            Some(TestMenu::Session(TestSession::Pause)) => loaded && !paused,
                            Some(TestMenu::Session(TestSession::Resume)) => loaded && paused,
                            Some(TestMenu::Session(TestSession::LoadActive)) => {
                                loaded && active.is_some()
                            }
                            Some(_) => loaded,
                        };
                        assert_eq!(leaf.is_enabled(), enabled, "{command:?}");
                    }
                    for activation in expected {
                        let native = item(menu, activation);
                        let routed = routes.iter().find(|(id, _)| id == native.id()).unwrap().1;
                        assert_eq!(routed, activation);
                        let HostEvent::Menu(transported) = HostEvent::Menu(routed) else {
                            unreachable!();
                        };
                        assert_eq!(transported, activation);
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
            let (mut loaded, mut paused, mut guard_open) = (false, false, false);
            let mut items: Vec<u64> = Vec::new();
            let mut active_item: Option<u64> = None;
            // One opaque-id table for the whole scenario: repeated builds
            // keep identities, like the view-side registry across frames.
            let commands = FakeCommands::new();
            sync_and_check(
                &commands,
                &mut menu,
                loaded,
                paused,
                guard_open,
                &items,
                active_item,
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
            let open = commands.reference(TestMenu::Open);
            assert!(matches!(
                recording.0.lock().unwrap().as_slice(),
                [HostEvent::Menu(received)] if *received == open
            ));
            assert_eq!(menu.menus[&menu_id::GROUP_A].text(), "Group A");
            assert_eq!(
                item(&menu, commands.reference(TestMenu::AuxGamma)).text(),
                "Aux Gamma"
            );
            loaded = true;
            items = vec![42, 7];
            active_item = Some(42);
            sync_and_check(
                &commands,
                &mut menu,
                loaded,
                paused,
                guard_open,
                &items,
                active_item,
                &bar,
            );
            paused = true;
            sync_and_check(
                &commands,
                &mut menu,
                loaded,
                paused,
                guard_open,
                &items,
                active_item,
                &bar,
            );
            guard_open = true;
            sync_and_check(
                &commands,
                &mut menu,
                loaded,
                paused,
                guard_open,
                &items,
                active_item,
                &bar,
            );
            guard_open = false;
            active_item = Some(7);
            sync_and_check(
                &commands,
                &mut menu,
                loaded,
                paused,
                guard_open,
                &items,
                active_item,
                &bar,
            );
            // Delete one item, then the last: removed IDs/commands must vanish,
            // while the surviving item and static items retain their IDs.
            items = vec![7];
            sync_and_check(
                &commands,
                &mut menu,
                loaded,
                paused,
                guard_open,
                &items,
                active_item,
                &bar,
            );
            items = Vec::new();
            active_item = None;
            sync_and_check(
                &commands,
                &mut menu,
                loaded,
                paused,
                guard_open,
                &items,
                active_item,
                &bar,
            );
            loaded = false;
            sync_and_check(
                &commands,
                &mut menu,
                loaded,
                paused,
                guard_open,
                &items,
                active_item,
                &bar,
            );

            // Root structure comes from Host, even when exercising a standard
            // application menu on this test platform rather than macOS.
            use wakui_core::chrome::menu::{
                NativeApplicationEntry as A, NativeApplicationMenu, NativeApplicationRole as R,
            };
            let mut root = canned_bar(&commands, loaded, paused, guard_open, &items, active_item);
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
                menu.menus[&menu_id::GROUP_A].id()
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
    use wakui_core::chrome::menu::Menubar;
    use wakui_core::input::event::EventSink;

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
            _guard_open: bool,
            _menu: &Menubar,
        ) {
        }

        pub(crate) fn clear_event_handler(&self) {}
    }

    impl super::MenuSink for AppMenu {
        fn update(&mut self, _loaded: bool, _paused: bool, menu: &Menubar) {
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
