//! Outer window traversal. Grouped contents keep their existing iced pipelines.

pub use crate::handler::WidgetBinding;
use iced_winit::program::Program;
use wakui::control::ControlScreen;
use wakui::{
    focus::AuxKind,
    window::{DebugContents, PpuContents, Window, WindowVisitor},
};

pub trait GroupedProgram: Program {
    const KIND: AuxKind;
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
