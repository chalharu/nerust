//! Platform handler interface: native widget state and callbacks.
//!
//! A backend [`WidgetBinding`] is hosted by the single native Program
//! adapter; aux views are bindings, not standalone programs.
//! This is the MAUI handler counterpart: the framework visits control
//! snapshots, the handler produces native elements.

use wakui_core::control::ControlScreen;
use wakui_core::window::Window;

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
