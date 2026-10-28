//! One native binding for every Host control session. No domain state lives here.
use std::{cell::RefCell, rc::Rc};

use iced::{Task, Theme};
use nerust_frontend_host::{focus::AuxKind, window::Window};
use wakui::Key;
use wakui::control::{ControlEvent, ControlScreen};

use crate::{aux::window::WidgetBinding, convert::default_font};

pub(crate) type Message = ControlEvent;
pub(crate) type El<'a> = iced::Element<'a, Message, Theme, iced_tiny_skia::Renderer>;
/// Screen snapshot shared between the iced state and the native handle.
pub(crate) type SharedScreen = Rc<RefCell<ControlScreen>>;
/// Control-event queue: iced update pushes here, the host drain consumes.
pub(crate) type EventQueue = Rc<RefCell<Vec<ControlEvent>>>;

pub(crate) struct ControlsState {
    pub(crate) screen: SharedScreen,
    pub(crate) queue: EventQueue,
}

pub(crate) struct ControlsBinding {
    // Boot transfers exactly the prepared screen, never recreating its draft.
    initial: RefCell<Option<(SharedScreen, EventQueue)>>,
    kind: AuxKind,
}

impl ControlsBinding {
    pub(crate) fn new(screen: ControlScreen, kind: AuxKind) -> (Self, SharedScreen, EventQueue) {
        let screen = Rc::new(RefCell::new(screen));
        let queue = Rc::new(RefCell::new(Vec::new()));
        (
            Self {
                initial: RefCell::new(Some((Rc::clone(&screen), Rc::clone(&queue)))),
                kind,
            },
            screen,
            queue,
        )
    }
}

impl WidgetBinding for ControlsBinding {
    type State = ControlsState;
    type Message = Message;
    type Theme = Theme;
    type Renderer = iced_tiny_skia::Renderer;
    type Executor = iced_winit::futures::backend::default::Executor;

    fn kind(&self) -> AuxKind {
        self.kind
    }

    fn settings(&self) -> iced::Settings {
        iced::Settings {
            default_font: default_font(),
            default_text_size: iced::Pixels(16.0),
            ..Default::default()
        }
    }

    fn boot(&self) -> (ControlsState, Task<Message>) {
        let (screen, queue) = self
            .initial
            .borrow_mut()
            .take()
            .expect("controls boot once");
        (ControlsState { screen, queue }, Task::none())
    }

    fn update(&self, state: &mut ControlsState, message: Message) -> Task<Message> {
        // Queue for the host drain: picker dialogs and submissions run
        // outside iced update, never in a nested native loop.
        state.queue.borrow_mut().push(message);
        Task::none()
    }

    fn presentation(&self, _state: &ControlsState) -> Option<Window> {
        None
    }

    fn view<'a>(&self, state: &'a ControlsState, _: iced::window::Id) -> El<'a> {
        let screen = state.screen.borrow();
        super::controls::build_screen(&screen)
    }

    fn controls_view<'a>(
        &self,
        state: &'a ControlsState,
        _screen: &ControlScreen,
        id: iced::window::Id,
    ) -> El<'a> {
        self.view(state, id)
    }
}

pub(crate) fn keyboard_key_from_physical(physical: iced::keyboard::key::Physical) -> Option<Key> {
    crate::keyboard::key_from_physical(physical)
}

#[cfg(test)]
pub(crate) mod tests;
