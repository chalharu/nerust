//! One native binding for every Host control session. No domain state lives here.
use std::{cell::RefCell, rc::Rc};

use iced::{Task, Theme};
use nerust_frontend_host::{
    control::{ControlEffect, ControlEvent, ControlScreen},
    focus::AuxKind,
    widget_session::WidgetSession,
    window::Window,
};
use nerust_keyboard::Key;

use crate::{aux::window::WidgetBinding, convert::default_font};

pub(crate) type Message = ControlEvent;
pub(crate) type El<'a> = iced::Element<'a, Message, Theme, iced_tiny_skia::Renderer>;
pub(crate) type ControlsState = Rc<RefCell<WidgetSession>>;

pub(crate) struct ControlsBinding {
    // Boot transfers exactly the prepared session, never recreating its draft.
    initial_state: RefCell<Option<ControlsState>>,
    kind: AuxKind,
}

impl ControlsBinding {
    pub(crate) fn new(session: WidgetSession) -> Self {
        let nerust_frontend_host::window::WindowId::Auxiliary(kind) = session.window().id else {
            unreachable!("widget sessions are auxiliary windows");
        };
        Self {
            initial_state: RefCell::new(Some(Rc::new(RefCell::new(session)))),
            kind,
        }
    }

    pub(crate) fn session(&self) -> ControlsState {
        self.initial_state
            .borrow()
            .as_ref()
            .expect("before boot")
            .clone()
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
        (
            self.initial_state
                .borrow_mut()
                .take()
                .expect("controls boot once"),
            Task::none(),
        )
    }

    fn update(&self, state: &mut ControlsState, message: Message) -> Task<Message> {
        // Dispatch ends before entering the native picker (which may run a nested loop).
        let effects = state.borrow_mut().dispatch(message);
        for effect in effects {
            let ControlEffect::PickDirectory {
                title,
                target,
                request,
            } = effect;
            let path = rfd::FileDialog::new().set_title(title).pick_folder();
            state.borrow_mut().dispatch(ControlEvent::new(
                target,
                nerust_frontend_host::control::ControlInput::DirectorySelected { request, path },
            ));
        }
        Task::none()
    }

    fn presentation(&self, state: &ControlsState) -> Option<Window> {
        Some(state.borrow().window())
    }

    fn view<'a>(&self, state: &'a ControlsState, _: iced::window::Id) -> El<'a> {
        let screen = state.borrow().screen();
        super::controls::build_screen(&screen)
    }

    fn controls_view<'a>(
        &self,
        _: &'a ControlsState,
        screen: &ControlScreen,
        _: iced::window::Id,
    ) -> El<'a> {
        super::controls::build_screen(screen)
    }
}

pub(crate) fn keyboard_key_from_physical(physical: iced::keyboard::key::Physical) -> Option<Key> {
    crate::keyboard::key_from_physical(physical)
}

#[cfg(test)]
pub(crate) mod tests;
