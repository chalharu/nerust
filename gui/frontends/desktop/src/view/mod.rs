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
    available: bool,
    loaded: bool,
    pause_enabled: bool,
    resume_enabled: bool,
    load_active_enabled: bool,
    slot_submenu_enabled: bool,
    settings_shortcut: Option<String>,
    quit_shortcut: Option<String>,
    file_title: String,
    emulation_title: String,
    quit_label: String,
    open_label: String,
    close_label: String,
    settings_label: String,
    pause_label: String,
    resume_label: String,
    reset_label: String,
    states_title: String,
    create_slot_label: String,
    save_active_slot_label: String,
    load_active_slot_label: String,
    select_slot_title: String,
    save_slot_title: String,
    load_slot_title: String,
    delete_slot_title: String,
    debugger_label: String,
    history_label: String,
    ppu_label: String,
    fullscreen_label: String,
    select_slot_nodes: Vec<MenuNode<Intent>>,
    save_slot_nodes: Vec<MenuNode<Intent>>,
    load_slot_nodes: Vec<MenuNode<Intent>>,
    delete_slot_nodes: Vec<MenuNode<Intent>>,
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
        let label = |key| text(language, key).to_owned();
        let hinted = |key, action| {
            nerust_gui_runtime::menu_labels::shortcut_hint(&label(key), &shortcuts, action)
        };
        // The application menu owns these accelerators where it exists;
        // registering the same accelerator twice leaves routing ambiguous.
        let settings_shortcut = Some("Primary+Comma".to_string());
        let quit_shortcut = Some("Primary+Q".to_string());
        let slot_nodes = |id: &str, action: fn(u64) -> SessionAction| {
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
                        visible: true,
                    })
                })
                .collect();
            if children.is_empty() {
                children.push(MenuNode::Item(MenuItem {
                    id: format!("{id}_empty"),
                    label: label(UiText::NoSaveSlots),
                    argument: Intent::Session(SessionAction::CreateSlot),
                    shortcut: None,
                    role: MenuRole::Normal,
                    enabled: false,
                    visible: true,
                }));
            }
            children
        };
        Self {
            state,
            language,
            shortcuts: shortcuts.clone(),
            slots: slots.clone(),
            active_slot,
            available,
            loaded,
            pause_enabled: loaded && !state.user_paused,
            resume_enabled: loaded && state.user_paused,
            load_active_enabled: loaded && active_slot.is_some(),
            slot_submenu_enabled: loaded,
            settings_shortcut,
            quit_shortcut,
            file_title: label(UiText::File),
            emulation_title: label(UiText::Emulation),
            quit_label: label(UiText::Quit),
            open_label: label(UiText::Open),
            close_label: label(UiText::Close),
            settings_label: label(UiText::Settings),
            pause_label: label(UiText::Pause),
            resume_label: label(UiText::Resume),
            reset_label: hinted(UiText::Reset, ShortcutAction::Reset),
            states_title: label(UiText::SaveStates),
            create_slot_label: label(UiText::CreateSaveSlot),
            save_active_slot_label: hinted(UiText::SaveActiveSlot, ShortcutAction::SaveActiveSlot),
            load_active_slot_label: hinted(UiText::LoadActiveSlot, ShortcutAction::LoadActiveSlot),
            select_slot_title: label(UiText::SelectActiveSlot),
            save_slot_title: label(UiText::SaveSlot),
            load_slot_title: label(UiText::LoadSlot),
            delete_slot_title: label(UiText::DeleteSlot),
            debugger_label: hinted(UiText::OpenDebugger, ShortcutAction::OpenDebugger),
            history_label: label(UiText::NotificationHistory),
            ppu_label: label(UiText::PpuViewer),
            fullscreen_label: hinted(UiText::FullscreenDefault, ShortcutAction::ToggleFullscreen),
            select_slot_nodes: slot_nodes("select_slot", SessionAction::SelectSlot),
            save_slot_nodes: slot_nodes("save_slot", SessionAction::SaveSlot),
            load_slot_nodes: slot_nodes("load_slot", SessionAction::LoadSlot),
            delete_slot_nodes: slot_nodes("delete_slot", SessionAction::DeleteSlot),
        }
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
    available: Binding<bool>,
    loaded: Binding<bool>,
    pause_enabled: Binding<bool>,
    resume_enabled: Binding<bool>,
    load_active_enabled: Binding<bool>,
    slot_submenu_enabled: Binding<bool>,
    settings_shortcut: Binding<Option<String>>,
    quit_shortcut: Binding<Option<String>>,
    file_title: Binding<String>,
    emulation_title: Binding<String>,
    quit_label: Binding<String>,
    open_label: Binding<String>,
    close_label: Binding<String>,
    settings_label: Binding<String>,
    pause_label: Binding<String>,
    resume_label: Binding<String>,
    reset_label: Binding<String>,
    states_title: Binding<String>,
    create_slot_label: Binding<String>,
    save_active_slot_label: Binding<String>,
    load_active_slot_label: Binding<String>,
    select_slot_title: Binding<String>,
    save_slot_title: Binding<String>,
    load_slot_title: Binding<String>,
    delete_slot_title: Binding<String>,
    debugger_label: Binding<String>,
    history_label: Binding<String>,
    ppu_label: Binding<String>,
    fullscreen_label: Binding<String>,
    select_slot_nodes: Binding<Vec<MenuNode<Intent>>>,
    save_slot_nodes: Binding<Vec<MenuNode<Intent>>>,
    load_slot_nodes: Binding<Vec<MenuNode<Intent>>>,
    delete_slot_nodes: Binding<Vec<MenuNode<Intent>>>,
    quit_arg: Binding<Intent>,
    open_arg: Binding<Intent>,
    close_arg: Binding<Intent>,
    settings_arg: Binding<Intent>,
    pause_arg: Binding<Intent>,
    resume_arg: Binding<Intent>,
    reset_arg: Binding<Intent>,
    create_slot_arg: Binding<Intent>,
    save_active_slot_arg: Binding<Intent>,
    load_active_slot_arg: Binding<Intent>,
    debugger_arg: Binding<Intent>,
    history_arg: Binding<Intent>,
    ppu_arg: Binding<Intent>,
    fullscreen_arg: Binding<Intent>,
    inert_arg: Binding<Intent>,
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
    fn constant<T: Clone + 'static>(value: T) -> Binding<T> {
        Binding::constant(value)
    }
    let bindings = Presentation {
        screen: model.bind(|model| model.screen.clone(), None),
        available: menus.bind(|model| model.available, None),
        loaded: menus.bind(|model| model.loaded, None),
        pause_enabled: menus.bind(|model| model.pause_enabled, None),
        resume_enabled: menus.bind(|model| model.resume_enabled, None),
        load_active_enabled: menus.bind(|model| model.load_active_enabled, None),
        slot_submenu_enabled: menus.bind(|model| model.slot_submenu_enabled, None),
        settings_shortcut: menus.bind(|model| model.settings_shortcut.clone(), None),
        quit_shortcut: menus.bind(|model| model.quit_shortcut.clone(), None),
        file_title: menus.bind(|model| model.file_title.clone(), None),
        emulation_title: menus.bind(|model| model.emulation_title.clone(), None),
        quit_label: menus.bind(|model| model.quit_label.clone(), None),
        open_label: menus.bind(|model| model.open_label.clone(), None),
        close_label: menus.bind(|model| model.close_label.clone(), None),
        settings_label: menus.bind(|model| model.settings_label.clone(), None),
        pause_label: menus.bind(|model| model.pause_label.clone(), None),
        resume_label: menus.bind(|model| model.resume_label.clone(), None),
        reset_label: menus.bind(|model| model.reset_label.clone(), None),
        states_title: menus.bind(|model| model.states_title.clone(), None),
        create_slot_label: menus.bind(|model| model.create_slot_label.clone(), None),
        save_active_slot_label: menus.bind(|model| model.save_active_slot_label.clone(), None),
        load_active_slot_label: menus.bind(|model| model.load_active_slot_label.clone(), None),
        select_slot_title: menus.bind(|model| model.select_slot_title.clone(), None),
        save_slot_title: menus.bind(|model| model.save_slot_title.clone(), None),
        load_slot_title: menus.bind(|model| model.load_slot_title.clone(), None),
        delete_slot_title: menus.bind(|model| model.delete_slot_title.clone(), None),
        debugger_label: menus.bind(|model| model.debugger_label.clone(), None),
        history_label: menus.bind(|model| model.history_label.clone(), None),
        ppu_label: menus.bind(|model| model.ppu_label.clone(), None),
        fullscreen_label: menus.bind(|model| model.fullscreen_label.clone(), None),
        select_slot_nodes: menus.bind(|model| model.select_slot_nodes.clone(), None),
        save_slot_nodes: menus.bind(|model| model.save_slot_nodes.clone(), None),
        load_slot_nodes: menus.bind(|model| model.load_slot_nodes.clone(), None),
        delete_slot_nodes: menus.bind(|model| model.delete_slot_nodes.clone(), None),
        quit_arg: constant(Intent::Quit),
        open_arg: constant(Intent::Open),
        close_arg: constant(Intent::Session(SessionAction::Unload)),
        settings_arg: constant(Intent::Settings),
        pause_arg: constant(Intent::Pause(true)),
        resume_arg: constant(Intent::Pause(false)),
        reset_arg: constant(Intent::Session(SessionAction::Reset)),
        create_slot_arg: constant(Intent::Session(SessionAction::CreateSlot)),
        save_active_slot_arg: constant(Intent::Session(SessionAction::SaveState)),
        load_active_slot_arg: constant(Intent::Session(SessionAction::LoadState)),
        debugger_arg: constant(Intent::Debugger),
        history_arg: constant(Intent::History),
        ppu_arg: constant(Intent::Ppu),
        fullscreen_arg: constant(Intent::Fullscreen),
        // Separators never dispatch; About is inert by construction (the
        // native panel owns it), so it fills the type without a meaning.
        inert_arg: constant(Intent::About),
        model,
        actions,
    };
    panechord::view!("ui/main.yaml", bindings, panechord::backend)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::test_fixture::loaded_adapter;

    #[test]
    fn skeleton_leaves_carry_localized_labels_and_gating() {
        let (adapter, _dir) = loaded_adapter();
        let app = nerust_gui_application::Application::new(adapter);
        let model = MenuModel::new(&app);
        assert_eq!(model.file_title, text(model.language, UiText::File));
        assert_eq!(
            model.emulation_title,
            text(model.language, UiText::Emulation)
        );
        // Loaded fixture with slots: creation and per-slot traffic enabled.
        assert!(model.available && model.loaded);
        assert_eq!(model.open_label, text(model.language, UiText::Open));
        assert_eq!(
            model.debugger_label,
            nerust_gui_runtime::menu_labels::shortcut_hint(
                text(model.language, UiText::OpenDebugger),
                &model.shortcuts,
                ShortcutAction::OpenDebugger,
            )
        );
        assert_eq!(model.pause_label, text(model.language, UiText::Pause));
        assert!(model.matches(&app));
    }

    #[test]
    fn accelerator_ownership_follows_application_menu_flag() {
        // The app resolves the flag at startup from the platform, so both
        // branches run on any platform without a macOS host.
        let (adapter, _dir) = loaded_adapter();
        let app = nerust_gui_application::Application::new(adapter);
        let model = MenuModel::new(&app);
        assert_eq!(model.settings_shortcut.as_deref(), Some("Primary+Comma"));
        assert_eq!(model.quit_shortcut.as_deref(), Some("Primary+Q"));
    }

    #[test]
    fn slot_subtrees_track_slots_and_placeholders() {
        use nerust_gui_application::SessionAction;
        let (adapter, _dir) = loaded_adapter();
        let app = nerust_gui_application::Application::new(adapter);
        // Fresh fixture: no slots yet, so the subtrees carry the disabled
        // placeholder with a create-slot fallback argument.
        let model = MenuModel::new(&app);
        assert!(model.slot_submenu_enabled, "submenus open when loaded");
        for nodes in [
            &model.select_slot_nodes,
            &model.save_slot_nodes,
            &model.load_slot_nodes,
            &model.delete_slot_nodes,
        ] {
            assert_eq!(nodes.len(), 1);
            let MenuNode::Item(item) = &nodes[0] else {
                panic!("placeholder is a leaf");
            };
            assert!(!item.enabled);
            assert_eq!(item.argument, Intent::Session(SessionAction::CreateSlot));
        }
    }
}
