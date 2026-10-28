//! Outer window traversal. Grouped contents keep their existing iced pipelines.

use iced_winit::program::Program;
use nerust_frontend_host::{
    focus::AuxKind,
    window::{DebugContents, PpuContents, Window, WindowVisitor},
};
use wakui::control::ControlScreen;

pub(crate) trait GroupedProgram: Program {
    const KIND: AuxKind;
}

/// Native widget state and callbacks, not a standalone iced application.
/// Settings and History are bindings hosted by this single Program adapter.
pub(crate) trait WidgetBinding {
    type State;
    type Message: Send + 'static;
    type Theme: iced::theme::Base;
    type Renderer: iced_winit::program::Renderer;
    type Executor: iced::Executor;
    fn kind(&self) -> AuxKind;

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

// Debug/PPU keep their established grouped pipelines.
impl<P: GroupedProgram> WidgetBinding for P {
    type State = P::State;
    type Message = P::Message;
    type Theme = P::Theme;
    type Renderer = P::Renderer;
    type Executor = P::Executor;
    fn kind(&self) -> AuxKind {
        P::KIND
    }
    fn settings(&self) -> iced::Settings {
        Program::settings(self)
    }
    fn boot(&self) -> (Self::State, iced::Task<Self::Message>) {
        Program::boot(self)
    }
    fn update(&self, state: &mut Self::State, message: Self::Message) -> iced::Task<Self::Message> {
        Program::update(self, state, message)
    }
    fn theme(&self, state: &Self::State, id: iced::window::Id) -> Option<Self::Theme> {
        Program::theme(self, state, id)
    }
    fn view<'a>(
        &self,
        state: &'a Self::State,
        id: iced::window::Id,
    ) -> iced::Element<'a, Self::Message, Self::Theme, Self::Renderer> {
        Program::view(self, state, id)
    }
}

pub(crate) struct WindowProgram<P> {
    pub(crate) window: Window,
    pub(crate) program: P,
}

impl<P: WidgetBinding> Program for WindowProgram<P> {
    type State = P::State;
    type Message = P::Message;
    type Theme = P::Theme;
    type Renderer = P::Renderer;
    type Executor = P::Executor;

    fn name() -> &'static str {
        "nerust_window"
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
        let mut visitor = GroupedView {
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
            .expect("window contents must match the native grouped program")
    }
}

struct GroupedView<'p, 'a, P: WidgetBinding> {
    program: &'p P,
    state: &'a P::State,
    id: iced::window::Id,
    output: Option<iced::Element<'a, P::Message, P::Theme, P::Renderer>>,
}

impl<P: WidgetBinding> GroupedView<'_, '_, P> {
    fn grouped(&mut self, kind: AuxKind) {
        if kind == self.program.kind() {
            self.output = Some(self.program.view(self.state, self.id));
        }
    }
}

impl<P: WidgetBinding> WindowVisitor for GroupedView<'_, '_, P> {
    fn visit_controls(&mut self, screen: &ControlScreen) {
        self.output = Some(self.program.controls_view(self.state, screen, self.id));
    }
    fn visit_debug(&mut self, _: &DebugContents) {
        self.grouped(AuxKind::Debug);
    }
    fn visit_ppu(&mut self, _: &PpuContents) {
        self.grouped(AuxKind::Ppu);
    }
}
