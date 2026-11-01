use nerust_gui_application::{Application, ApplicationState, SessionAction};
use nerust_settings_core::{
    i18n::{UiText, text},
    model::{
        input::{ShortcutAction, ShortcutBinding},
        language::AppLanguage,
    },
};
use panechord::{Binding, Command, ImageFrame, MenuItem, MenuNode, MenuRole, Parent, Store, View};
use std::{cell::RefCell, rc::Rc, sync::Arc};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Intent {
    Open,
    Settings,
    History,
    Debugger,
    Ppu,
    About,
    Pause(bool),
    Session(SessionAction),
    Fullscreen,
    Quit,
}
pub(crate) type Dispatch = Rc<dyn Fn(Intent) -> bool>;
pub(crate) type Actions = Rc<RefCell<Option<Dispatch>>>;

#[derive(Clone)]
pub(crate) struct MenuModel {
    state: ApplicationState,
    language: AppLanguage,
    shortcuts: Vec<ShortcutBinding>,
    slots: Arc<[nerust_persistence::model::StateSlotSummary]>,
    active_slot: Option<u64>,
    app_title: String,
    file_title: String,
    emulation_title: String,
    app: Vec<MenuNode<Intent>>,
    file: Vec<MenuNode<Intent>>,
    emulation: Vec<MenuNode<Intent>>,
}

impl MenuModel {
    pub(crate) fn matches(&self, app: &Application<crate::SessionAdapter>) -> bool {
        let (slots, active_slot) = app.session().menu_slots();
        self.state == app.state()
            && self.language == app.settings().shared.general.language
            && self.shortcuts == app.settings().shared.input.shortcuts.keyboard
            && self.active_slot == active_slot
            && Arc::ptr_eq(&self.slots, &slots)
    }

    pub(crate) fn new(app: &Application<crate::SessionAdapter>) -> Self {
        let state = app.state();
        let language = app.settings().shared.general.language;
        let shortcuts = app.settings().shared.input.shortcuts.keyboard.clone();
        let (slots, active_slot) = app.session().menu_slots();
        let available = !state.closed;
        let loaded = available && state.loaded;
        let item = |id: &str, caption, intent, enabled, shortcut| {
            MenuNode::Item(MenuItem {
                id: id.into(),
                label: text(language, caption).into(),
                argument: intent,
                shortcut,
                role: MenuRole::Normal,
                enabled,
            })
        };
        let mut file = vec![
            item(
                "open",
                UiText::Open,
                Intent::Open,
                available,
                Some("Primary+O".into()),
            ),
            item(
                "close",
                UiText::Close,
                Intent::Session(SessionAction::Unload),
                loaded,
                None,
            ),
            item(
                "settings",
                UiText::Settings,
                Intent::Settings,
                available,
                // macOS carries Primary+Comma on the app-menu Preferences entry;
                // registering the same accelerator twice leaves routing ambiguous.
                (!panechord::wants_application_menu()).then(|| "Primary+Comma".into()),
            ),
            item(
                "quit",
                UiText::Quit,
                Intent::Quit,
                available,
                // macOS shows Quit with Primary+Q in the app menu; see above.
                (!panechord::wants_application_menu()).then(|| "Primary+Q".into()),
            ),
        ];
        if let MenuNode::Item(quit) = &mut file[3] {
            quit.role = MenuRole::Quit;
        }
        let app = Self::app_menu(language, available);
        let hinted = |id: &str, caption, action, intent, enabled| {
            let mut node = item(id, caption, intent, enabled, None);
            if let MenuNode::Item(item) = &mut node {
                item.label =
                    nerust_gui_runtime::menu_labels::shortcut_hint(&item.label, &shortcuts, action);
            }
            node
        };
        let slot_menu = |id: &str, caption, action: fn(u64) -> SessionAction| {
            let mut children: Vec<_> = slots
                .iter()
                .map(|slot| {
                    MenuNode::Item(MenuItem {
                        id: format!("{id}_{}", slot.slot_id),
                        label: nerust_gui_runtime::slots::slot_label(slot, active_slot),
                        argument: Intent::Session(action(slot.slot_id)),
                        shortcut: None,
                        role: MenuRole::Normal,
                        enabled: loaded,
                    })
                })
                .collect();
            if children.is_empty() {
                children.push(item(
                    &format!("{id}_empty"),
                    UiText::NoSaveSlots,
                    Intent::Session(SessionAction::CreateSlot),
                    false,
                    None,
                ));
            }
            MenuNode::Submenu {
                id: id.into(),
                label: text(language, caption).into(),
                enabled: loaded,
                children,
            }
        };
        let states = MenuNode::Submenu {
            id: "save_states".into(),
            label: text(language, UiText::SaveStates).into(),
            enabled: loaded,
            children: vec![
                item(
                    "create_slot",
                    UiText::CreateSaveSlot,
                    Intent::Session(SessionAction::CreateSlot),
                    loaded,
                    None,
                ),
                hinted(
                    "save_active",
                    UiText::SaveActiveSlot,
                    ShortcutAction::SaveActiveSlot,
                    Intent::Session(SessionAction::SaveState),
                    loaded,
                ),
                hinted(
                    "load_active",
                    UiText::LoadActiveSlot,
                    ShortcutAction::LoadActiveSlot,
                    Intent::Session(SessionAction::LoadState),
                    loaded && active_slot.is_some(),
                ),
                slot_menu(
                    "select_slot",
                    UiText::SelectActiveSlot,
                    SessionAction::SelectSlot,
                ),
                slot_menu("save_slot", UiText::SaveSlot, SessionAction::SaveSlot),
                slot_menu("load_slot", UiText::LoadSlot, SessionAction::LoadSlot),
                slot_menu("delete_slot", UiText::DeleteSlot, SessionAction::DeleteSlot),
            ],
        };
        let emulation = vec![
            item(
                "pause",
                UiText::Pause,
                Intent::Pause(true),
                loaded && !state.user_paused,
                None,
            ),
            item(
                "resume",
                UiText::Resume,
                Intent::Pause(false),
                loaded && state.user_paused,
                None,
            ),
            hinted(
                "reset",
                UiText::Reset,
                ShortcutAction::Reset,
                Intent::Session(SessionAction::Reset),
                loaded,
            ),
            states,
            hinted(
                "debugger",
                UiText::OpenDebugger,
                ShortcutAction::OpenDebugger,
                Intent::Debugger,
                loaded,
            ),
            item(
                "history",
                UiText::NotificationHistory,
                Intent::History,
                available,
                None,
            ),
            item("ppu", UiText::PpuViewer, Intent::Ppu, loaded, None),
            hinted(
                "fullscreen",
                UiText::FullscreenDefault,
                ShortcutAction::ToggleFullscreen,
                Intent::Fullscreen,
                available,
            ),
        ];
        Self {
            state,
            language,
            shortcuts,
            slots,
            active_slot,
            app_title: Self::app_title(),
            file_title: text(language, UiText::File).into(),
            emulation_title: text(language, UiText::Emulation).into(),
            app,
            file,
            emulation,
        }
    }

    fn app_title() -> String {
        "Nerust".into()
    }

    /// Explicit HIG application menu for macOS: AppKit treats the first
    /// installed submenu as the application menu, so without this leading
    /// entry the File menu is swallowed. About is a native predefined item
    /// (the OS owns its panel, so `Intent::About` never dispatches);
    /// Preferences and Quit reuse the existing intents. Single yaml for all
    /// platforms: off macOS this stays empty and backends skip the vacancy.
    fn app_menu(language: AppLanguage, available: bool) -> Vec<MenuNode<Intent>> {
        if !panechord::wants_application_menu() {
            return Vec::new();
        }
        let separator = |id: &str| {
            MenuNode::Item(MenuItem {
                id: id.into(),
                label: String::new(),
                // Inert: separators never dispatch; any intent fills the type.
                argument: Intent::About,
                shortcut: None,
                role: MenuRole::Separator,
                enabled: true,
            })
        };
        vec![
            MenuNode::Item(MenuItem {
                id: "about".into(),
                label: text(language, UiText::About).into(),
                argument: Intent::About,
                shortcut: None,
                role: MenuRole::About,
                enabled: available,
            }),
            separator("about_separator"),
            MenuNode::Item(MenuItem {
                id: "preferences".into(),
                label: text(language, UiText::Preferences).into(),
                argument: Intent::Settings,
                shortcut: Some("Primary+Comma".into()),
                role: MenuRole::Normal,
                enabled: available,
            }),
            separator("preferences_separator"),
            MenuNode::Item(MenuItem {
                id: "quit".into(),
                label: text(language, UiText::Quit).into(),
                argument: Intent::Quit,
                shortcut: Some("Primary+Q".into()),
                role: MenuRole::Quit,
                enabled: available,
            }),
        ]
    }
}

#[derive(Clone)]
pub(crate) struct Model {
    pub screen: Option<ImageFrame>,
    pub state: ApplicationState,
    pub notice: Option<String>,
}
impl Model {
    pub fn new(state: ApplicationState) -> Self {
        Self {
            screen: None,
            state,
            notice: None,
        }
    }
}
pub(crate) fn screen(view: &View, parent: &Parent, id: &str, source: Binding<Option<ImageFrame>>) {
    view.surface(parent, id, source);
}

#[derive(Clone)]
struct Presentation {
    screen: Binding<Option<ImageFrame>>,
    app_title: Binding<String>,
    file_title: Binding<String>,
    emulation_title: Binding<String>,
    app_items: Binding<Vec<MenuNode<Intent>>>,
    file_items: Binding<Vec<MenuNode<Intent>>>,
    emulation_items: Binding<Vec<MenuNode<Intent>>>,
    model: Store<Model>,
    actions: Actions,
}
impl Presentation {
    fn command_choose(&self, argument: Binding<Intent>) -> Command {
        let actions = self.actions.clone();
        self.model.command(
            |model| !model.state.closed,
            move |_, intent: Intent| {
                let dispatch = actions.borrow().clone();
                if let Some(dispatch) = dispatch {
                    dispatch(intent);
                }
            },
            argument,
        )
    }
}
pub(crate) fn build(model: Store<Model>, menus: Store<MenuModel>, actions: Actions) -> View {
    let bindings = Presentation {
        screen: model.bind(|model| model.screen.clone(), None),
        app_title: menus.bind(|model| model.app_title.clone(), None),
        file_title: menus.bind(|model| model.file_title.clone(), None),
        emulation_title: menus.bind(|model| model.emulation_title.clone(), None),
        app_items: menus.bind(|model| model.app.clone(), None),
        file_items: menus.bind(|model| model.file.clone(), None),
        emulation_items: menus.bind(|model| model.emulation.clone(), None),
        model,
        actions,
    };
    panechord::view!("ui/main.yaml", bindings, panechord)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::test_fixture::loaded_adapter;

    fn items_of(menu: &[MenuNode<Intent>]) -> Vec<&MenuItem<Intent>> {
        menu.iter()
            .filter_map(|node| match node {
                MenuNode::Item(item) => Some(item),
                MenuNode::Submenu { .. } => None,
            })
            .collect()
    }

    fn file_settings_shortcut(model: &MenuModel) -> Option<&str> {
        items_of(&model.file)
            .iter()
            .find(|item| item.argument == Intent::Settings)
            .expect("File carries a Settings entry")
            .shortcut
            .as_deref()
    }

    #[test]
    fn file_and_emulation_menus_keep_shape_on_all_platforms() {
        let (adapter, _dir) = loaded_adapter();
        let app = nerust_gui_application::Application::new(adapter);
        let model = MenuModel::new(&app);
        assert_eq!(model.file_title, text(model.language, UiText::File));
        assert_eq!(
            model.emulation_title,
            text(model.language, UiText::Emulation)
        );
        assert_eq!(model.file.len(), 4, "Open/Close/Settings/Quit");
        // Loaded fixture: pause gated off (running), resume gated on... the
        // adapter boots paused-debug? Only shape is pinned here, not gating.
        assert!(!model.emulation.is_empty());
        assert!(model.matches(&app));
    }

    #[test]
    fn app_menu_is_vacant_off_macos_and_leads_on_macos() {
        let (adapter, _dir) = loaded_adapter();
        let app = nerust_gui_application::Application::new(adapter);
        let model = MenuModel::new(&app);
        if panechord::wants_application_menu() {
            assert_eq!(model.app_title, "Nerust");
            let items = items_of(&model.app);
            assert_eq!(items.len(), 5, "About/separator/Preferences/separator/Quit");
            assert_eq!(items[0].role, MenuRole::About);
            assert_eq!(items[0].argument, Intent::About);
            assert_eq!(items[1].role, MenuRole::Separator);
            assert_eq!(items[2].argument, Intent::Settings);
            assert_eq!(items[2].role, MenuRole::Normal);
            assert_eq!(items[2].shortcut.as_deref(), Some("Primary+Comma"));
            assert_eq!(items[3].role, MenuRole::Separator);
            assert_eq!(items[4].argument, Intent::Quit);
            assert_eq!(items[4].role, MenuRole::Quit);
            assert_eq!(items[4].shortcut.as_deref(), Some("Primary+Q"));
            // The app menu owns the accelerators: File must not duplicate them.
            assert_eq!(file_settings_shortcut(&model), None);
            let file = items_of(&model.file);
            let quit = file
                .iter()
                .find(|item| item.argument == Intent::Quit)
                .expect("File keeps Quit");
            assert_eq!(quit.shortcut, None);
        } else {
            assert!(model.app.is_empty(), "no app menu off macOS");
            assert_eq!(file_settings_shortcut(&model), Some("Primary+Comma"));
            let file = items_of(&model.file);
            let quit = file
                .iter()
                .find(|item| item.argument == Intent::Quit)
                .expect("File keeps Quit");
            assert_eq!(quit.shortcut.as_deref(), Some("Primary+Q"));
        }
    }
}
