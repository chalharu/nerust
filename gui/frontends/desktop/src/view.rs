use nerust_gui_application::ApplicationState;
use panechord::{Binding, ImageFrame, Parent, Store, View};
use std::rc::Rc;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Intent {
    Open,
    Settings,
    TogglePause,
    Session(nerust_gui_application::SessionAction),
    Fullscreen,
    Quit,
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

pub(crate) fn build(model: Store<Model>) -> View {
    panechord::view!("ui/main.yaml", model, panechord)
}

pub(crate) fn menus(view: &View, model: &Store<Model>, dispatch: Rc<dyn Fn(Intent) -> bool>) {
    use nerust_gui_application::SessionAction;
    use panechord::{MenuItem, MenuRole};
    let item = |id: &str, label: &str, intent, shortcut: Option<&str>, role, enabled| MenuItem {
        id: id.into(),
        label: label.into(),
        argument: intent,
        shortcut: shortcut.map(str::to_owned),
        role,
        enabled,
    };
    let make = Rc::new(item);
    let file = make.clone();
    view.typed_menu(
        "file",
        Binding::constant("File".into()),
        model.bind(
            move |_| {
                vec![
                    file(
                        "open",
                        "Open...",
                        Intent::Open,
                        Some("Primary+O"),
                        MenuRole::Normal,
                        true,
                    ),
                    file(
                        "quit",
                        "Quit",
                        Intent::Quit,
                        Some("Primary+Q"),
                        MenuRole::Quit,
                        true,
                    ),
                    file(
                        "settings",
                        "Settings...",
                        Intent::Settings,
                        Some("Primary+Comma"),
                        MenuRole::Normal,
                        true,
                    ),
                ]
            },
            None,
        ),
        dispatch.clone(),
    );
    let emulation = make.clone();
    view.typed_menu(
        "emulation",
        Binding::constant("Emulation".into()),
        model.bind(
            move |model| {
                let loaded = model.state.loaded;
                vec![
                    emulation(
                        "pause",
                        if model.state.user_paused {
                            "Resume"
                        } else {
                            "Pause"
                        },
                        Intent::TogglePause,
                        None,
                        MenuRole::Normal,
                        loaded,
                    ),
                    emulation(
                        "reset",
                        "Reset",
                        Intent::Session(SessionAction::Reset),
                        None,
                        MenuRole::Normal,
                        loaded,
                    ),
                    emulation(
                        "save",
                        "Save State",
                        Intent::Session(SessionAction::SaveState),
                        None,
                        MenuRole::Normal,
                        loaded,
                    ),
                    emulation(
                        "load",
                        "Load State",
                        Intent::Session(SessionAction::LoadState),
                        None,
                        MenuRole::Normal,
                        loaded,
                    ),
                    emulation(
                        "next",
                        "Next Slot",
                        Intent::Session(SessionAction::NextSlot),
                        None,
                        MenuRole::Normal,
                        loaded,
                    ),
                    emulation(
                        "previous",
                        "Previous Slot",
                        Intent::Session(SessionAction::PreviousSlot),
                        None,
                        MenuRole::Normal,
                        loaded,
                    ),
                    emulation(
                        "unload",
                        "Unload",
                        Intent::Session(SessionAction::Unload),
                        None,
                        MenuRole::Normal,
                        loaded,
                    ),
                ]
            },
            None,
        ),
        dispatch.clone(),
    );
    view.typed_menu(
        "view",
        Binding::constant("View".into()),
        model.bind(
            move |_| {
                vec![make(
                    "fullscreen",
                    "Fullscreen",
                    Intent::Fullscreen,
                    None,
                    MenuRole::Normal,
                    true,
                )]
            },
            None,
        ),
        dispatch,
    );
}
