//! Outer window traversal. All aux windows are generic controls windows.

pub use crate::handler::WidgetBinding;
use iced_winit::program::Program;
use wakui_core::control::ControlScreen;
use wakui_core::window::{Window, WindowVisitor};

pub struct WindowProgram<P> {
    pub window: Window,
    pub program: P,
}

impl<P: WidgetBinding> Program for WindowProgram<P> {
    type State = P::State;
    type Message = P::Message;
    type Theme = P::Theme;
    type Renderer = P::Renderer;
    type Executor = P::Executor;

    fn name() -> &'static str {
        "wakui_window"
    }
    fn settings(&self) -> iced::Settings {
        self.program.settings()
    }
    fn window(&self) -> Option<iced::window::Settings> {
        None
    }
    fn boot(&self) -> (Self::State, iced::Task<Self::Message>) {
        self.program.boot()
    }
    fn update(&self, state: &mut Self::State, message: Self::Message) -> iced::Task<Self::Message> {
        self.program.update(state, message)
    }
    fn theme(&self, state: &Self::State, id: iced::window::Id) -> Option<Self::Theme> {
        self.program.theme(state, id)
    }
    fn title(&self, state: &Self::State, _id: iced::window::Id) -> String {
        self.program
            .presentation(state)
            .unwrap_or_else(|| self.window.clone())
            .title
    }
    fn view<'a>(
        &self,
        state: &'a Self::State,
        id: iced::window::Id,
    ) -> iced::Element<'a, Self::Message, Self::Theme, Self::Renderer> {
        let mut visitor = ControlsView {
            program: &self.program,
            state,
            id,
            output: None,
        };
        self.program
            .presentation(state)
            .as_ref()
            .unwrap_or(&self.window)
            .visit(&mut visitor);
        visitor
            .output
            .expect("aux window must hold a prepared control screen")
    }
}

struct ControlsView<'p, 'a, P: WidgetBinding> {
    program: &'p P,
    state: &'a P::State,
    id: iced::window::Id,
    output: Option<iced::Element<'a, P::Message, P::Theme, P::Renderer>>,
}

impl<P: WidgetBinding> WindowVisitor for ControlsView<'_, '_, P> {
    fn visit_controls(&mut self, screen: &ControlScreen) {
        self.output = Some(self.program.controls_view(self.state, screen, self.id));
    }
}
