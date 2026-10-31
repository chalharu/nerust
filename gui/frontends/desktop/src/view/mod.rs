use nerust_gui_application::{ApplicationState, SessionAction};
use panechord::{Binding, Command, ImageFrame, Parent, Store, View};
use std::{cell::RefCell, rc::Rc};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Intent {
    Open,
    Settings,
    TogglePause,
    Session(SessionAction),
    Fullscreen,
    Quit,
}
pub(crate) type Dispatch = Rc<dyn Fn(Intent) -> bool>;
pub(crate) type Actions = Rc<RefCell<Option<Dispatch>>>;

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
    loaded: Binding<bool>,
    pause_label: Binding<String>,
    model: Store<Model>,
    actions: Actions,
}
impl Presentation {
    fn command_choose(&self, argument: Binding<String>) -> Command {
        let actions = self.actions.clone();
        self.model.command(
            |model| !model.state.closed,
            move |_, action: String| {
                let intent = match action.as_str() {
                    "open" => Intent::Open,
                    "settings" => Intent::Settings,
                    "pause" => Intent::TogglePause,
                    "fullscreen" => Intent::Fullscreen,
                    "quit" => Intent::Quit,
                    "reset" => Intent::Session(SessionAction::Reset),
                    "save" => Intent::Session(SessionAction::SaveState),
                    "load" => Intent::Session(SessionAction::LoadState),
                    "next" => Intent::Session(SessionAction::NextSlot),
                    "previous" => Intent::Session(SessionAction::PreviousSlot),
                    "unload" => Intent::Session(SessionAction::Unload),
                    _ => return,
                };
                let dispatch = actions.borrow().clone();
                if let Some(dispatch) = dispatch {
                    dispatch(intent);
                }
            },
            argument,
        )
    }
}
pub(crate) fn build(model: Store<Model>, actions: Actions) -> View {
    let bindings = Presentation {
        screen: model.bind(|model| model.screen.clone(), None),
        loaded: model.bind(|model| model.state.loaded, None),
        pause_label: model.bind(
            |model| {
                if model.state.user_paused {
                    "Resume"
                } else {
                    "Pause"
                }
                .into()
            },
            None,
        ),
        model,
        actions,
    };
    panechord::view!("ui/main.yaml", bindings, panechord)
}
