//! Platform handler interface: native widget state and callbacks.
//!
//! A backend [`WidgetBinding`] is hosted by the single native Program
//! adapter; aux views are bindings, not standalone programs.
//! This is the MAUI handler counterpart: the framework visits control
//! snapshots, the handler produces native elements.

use wakui_core::chrome::window::Window;
use wakui_core::scene::control::ControlScreen;

pub trait WidgetBinding {
    type State;
    type Message: Send + 'static;
    type Theme: iced::theme::Base;
    type Renderer: iced_winit::program::Renderer;
    type Executor: iced::Executor;

    fn settings(&self) -> iced::Settings;
    fn boot(&self) -> (Self::State, iced::Task<Self::Message>);
    fn update(&self, state: &mut Self::State, message: Self::Message) -> iced::Task<Self::Message>;
    fn theme(&self, _state: &Self::State, _id: iced::window::Id) -> Option<Self::Theme> {
        None
    }
    fn presentation(&self, _state: &Self::State) -> Option<Window> {
        None
    }
    fn view<'a>(
        &self,
        state: &'a Self::State,
        id: iced::window::Id,
    ) -> iced::Element<'a, Self::Message, Self::Theme, Self::Renderer>;
    fn controls_view<'a>(
        &self,
        state: &'a Self::State,
        _screen: &ControlScreen,
        id: iced::window::Id,
    ) -> iced::Element<'a, Self::Message, Self::Theme, Self::Renderer> {
        self.view(state, id)
    }
}

// One native binding for every Host control session. No domain state lives here.
use std::{cell::RefCell, rc::Rc};

use iced::{Task, Theme};
use wakui_core::Key;
use wakui_core::scene::control::ControlEvent;

use crate::input::convert::default_font;

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
    /// Test-only handoff probe; native hosts never read this.
    #[cfg(test)]
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
        crate::scene::render::build_screen(&screen)
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
    crate::input::keyboard::key_from_physical(physical)
}

#[cfg(test)]
mod binding_tests {
    use super::*;

    #[test]
    fn boot_moves_the_prepared_screen_and_queue_exactly_once() {
        let screen = wakui_core::support::conformance::conformance_screen();
        let (binding, shared, queue) = ControlsBinding::new(screen);
        assert!(binding.initial.borrow().is_some());
        let (mut state, _) = binding.boot();
        assert!(binding.initial.borrow().is_none());
        assert!(std::rc::Rc::ptr_eq(&shared, &state.screen));
        assert!(queue.borrow().is_empty());
        drop(binding.update(
            &mut state,
            ControlEvent::capture_key(wakui_core::Key::Enter),
        ));
        assert_eq!(queue.borrow().len(), 1);
    }
}
