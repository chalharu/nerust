//! One native binding for every Host control session. No domain state lives here.
use std::{cell::RefCell, rc::Rc};

use iced::{Task, Theme};
use wakui::Key;
use wakui::control::{ControlEvent, ControlScreen};
use wakui::window::Window;

use crate::{convert::default_font, handler::WidgetBinding};

pub type Message = ControlEvent;
pub type El<'a> = iced::Element<'a, Message, Theme, iced_tiny_skia::Renderer>;
/// Screen snapshot shared between the iced state and the native handle.
pub type SharedScreen = Rc<RefCell<ControlScreen>>;
/// Control-event queue: iced update pushes here, the host drain consumes.
pub type EventQueue = Rc<RefCell<Vec<ControlEvent>>>;

pub struct ControlsState {
    pub screen: SharedScreen,
    pub queue: EventQueue,
}

pub struct ControlsBinding {
    // Boot transfers exactly the prepared screen, never recreating its draft.
    initial: RefCell<Option<(SharedScreen, EventQueue)>>,
}

impl ControlsBinding {
    pub fn new(screen: ControlScreen) -> (Self, SharedScreen, EventQueue) {
        let screen = Rc::new(RefCell::new(screen));
        let queue = Rc::new(RefCell::new(Vec::new()));
        (
            Self {
                initial: RefCell::new(Some((Rc::clone(&screen), Rc::clone(&queue)))),
            },
            screen,
            queue,
        )
    }

    /// Whether the prepared screen is still waiting for its one boot move.
    /// Native hosts assert handoff without touching the queued state.
    pub fn boot_pending(&self) -> bool {
        self.initial.borrow().is_some()
    }
}

impl WidgetBinding for ControlsBinding {
    type State = ControlsState;
    type Message = Message;
    type Theme = Theme;
    type Renderer = iced_tiny_skia::Renderer;
    type Executor = iced_winit::futures::backend::default::Executor;

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

pub fn keyboard_key_from_physical(physical: iced::keyboard::key::Physical) -> Option<Key> {
    crate::keyboard::key_from_physical(physical)
}

#[cfg(test)]
mod binding_tests {
    use super::*;

    #[test]
    fn boot_moves_the_prepared_screen_and_queue_exactly_once() {
        let screen = wakui::conformance::conformance_screen();
        let (binding, shared, queue) = ControlsBinding::new(screen);
        assert!(binding.initial.borrow().is_some());
        let (mut state, _) = binding.boot();
        assert!(binding.initial.borrow().is_none());
        assert!(std::rc::Rc::ptr_eq(&shared, &state.screen));
        assert!(queue.borrow().is_empty());
        drop(binding.update(&mut state, ControlEvent::capture_key(wakui::Key::Enter)));
        assert_eq!(queue.borrow().len(), 1);
    }
}
