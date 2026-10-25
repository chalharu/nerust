//! Program pump: retained tree plus incoming message iterators, no live interface.
//! Relayout, pump, and message-run passes live here; surfaces live in `surface`.

use std::sync::atomic::{AtomicBool, Ordering};

use iced::{Size, mouse};
use iced_tiny_skia::Renderer;
use iced_winit::{
    Clipboard,
    program::{self, Program, core::window::RedrawRequest},
    runtime::user_interface::{Cache, State, UserInterface},
};

/// Retained program tree and layout cache.
/// No interface is retained across calls.
pub(crate) struct ViewTree<P: Program<Theme = iced::Theme, Renderer = Renderer>> {
    pub(super) instance: Box<program::Instance<P>>,
    pub(super) cache: Cache,
    pub(super) window_id: iced::window::Id,
    pub(super) bounds: Size,
    /// SPIKE NER-335: client-rect origin (logical px) for padded layout.
    /// `None` means the surface origin is the client origin (no CSD).
    pub(super) offset: Option<(f32, f32)>,
}

/// Program UI shell: retained tree plus reusable event-message scratch.
/// No interface is retained across calls.
pub(crate) struct UiShell<P: Program<Theme = iced::Theme, Renderer = Renderer>> {
    pub(super) tree: ViewTree<P>,
    /// Reused event-draw scratch: pumps and redraws never allocate per event.
    scratch: Vec<P::Message>,
    pub(super) deadline: Option<std::time::Instant>,
}

/// What a pumped event needs after `update`: nothing, a repaint,
/// or a view rebuild (`State::Outdated`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PumpOutcome {
    Idle,
    Repaint,
    Rebuild,
    At(std::time::Instant),
}

/// Pump policy: map iced's update result onto [`PumpOutcome`]. Pure, so tests cover it.
pub(crate) fn pump_outcome(state: &State) -> PumpOutcome {
    match state {
        State::Outdated => PumpOutcome::Rebuild,
        State::Updated {
            redraw_request: RedrawRequest::At(deadline),
            ..
        } => PumpOutcome::At(*deadline),
        State::Updated { redraw_request, .. } if *redraw_request != RedrawRequest::Wait => {
            PumpOutcome::Repaint
        }
        State::Updated { .. } => PumpOutcome::Idle,
    }
}

/// Relayout is infallible: recording new bounds is a plain store.
impl<P: Program<Theme = iced::Theme, Renderer = Renderer>> UiShell<P> {
    pub(crate) fn new(
        instance: program::Instance<P>,
        window_id: iced::window::Id,
        bounds: Size,
    ) -> Self {
        Self {
            tree: ViewTree {
                instance: Box::new(instance),
                cache: Cache::default(),
                window_id,
                bounds,
                offset: None,
            },
            scratch: Vec::new(),
            deadline: None,
        }
    }

    /// Record new layout bounds. Infallible: the next pump builds against them.
    pub(crate) fn relayout(&mut self, bounds: Size) {
        self.tree.bounds = bounds;
    }

    /// Record the client-rect origin for padded layout (SPIKE NER-335).
    /// Infallible: the next pump builds against it.
    pub(crate) fn set_content_offset(&mut self, offset: Option<(f32, f32)>) {
        self.tree.offset = offset;
    }

    /// Take the reused scratch buffer for one pump pass.
    pub(super) fn take_scratch(&mut self) -> Vec<P::Message> {
        std::mem::take(&mut self.scratch)
    }

    /// Return the scratch buffer after a pump pass.
    pub(super) fn restore_scratch(&mut self, mut out: Vec<P::Message>) {
        out.clear();
        self.scratch = out;
    }

    /// Build the transient interface for one pump pass.
    /// The returned interface borrows the instance; callers pass disjoint tree fields.
    /// SPIKE NER-335: with a client offset, content is padded to the client
    /// rect so the shared-surface origin stays clear for CSD chrome.
    #[allow(clippy::too_many_arguments)]
    fn build_transient<'a>(
        instance: &'a program::Instance<P>,
        window_id: iced::window::Id,
        bounds: Size,
        offset: Option<(f32, f32)>,
        cache: &mut Cache,
        renderer: &mut Renderer,
    ) -> UserInterface<'a, P::Message, iced::Theme, Renderer> {
        let cache = std::mem::take(cache);
        let element = instance.view(window_id);
        let element = match offset {
            Some((left, top)) => iced::widget::container(element)
                .padding(iced::Padding {
                    top,
                    right: 0.0,
                    bottom: 0.0,
                    left,
                })
                .width(iced::Length::Fill)
                .height(iced::Length::Fill)
                .into(),
            None => element,
        };
        UserInterface::build(element, bounds, cache, renderer)
    }

    /// Fold a finished pass back into the retained cache.
    fn retire_transient(
        cache: &mut Cache,
        ui: UserInterface<'_, P::Message, iced::Theme, Renderer>,
    ) {
        *cache = ui.into_cache();
    }

    /// What a pumped event needs after `update`: nothing, a repaint, or a view rebuild.
    /// Reuses the caller's `out` buffer (no per-event alloc).
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn pump(
        &mut self,
        mapped: iced::Event,
        cursor: mouse::Cursor,
        clipboard: &mut Clipboard,
        backend: &mut Renderer,
        out: &mut Vec<P::Message>,
        draw: Option<(&iced::Theme, &iced::advanced::renderer::Style)>,
    ) -> PumpOutcome {
        out.clear();
        if matches!(
            mapped,
            iced::Event::Window(iced::window::Event::RedrawRequested(_))
        ) {
            self.deadline = None;
        }
        // Disjoint tree fields: the interface borrows the instance
        // while the cache slot is taken out and folded back.
        let mut ui = Self::build_transient(
            &self.tree.instance,
            self.tree.window_id,
            self.tree.bounds,
            self.tree.offset,
            &mut self.tree.cache,
            backend,
        );
        let (state, _) = ui.update(&[mapped], cursor, backend, clipboard, out);
        let outcome = pump_outcome(&state);
        if let PumpOutcome::At(deadline) = outcome {
            self.deadline = Some(
                self.deadline
                    .map_or(deadline, |pending| pending.min(deadline)),
            );
        }
        if let Some((theme, style)) = draw {
            // Keep overlay layout and widget state from this update for this draw.
            ui.draw(backend, theme, style, cursor);
        }
        Self::retire_transient(&mut self.tree.cache, ui);
        outcome
    }

    /// Runs incoming messages directly, marking the retained cache dirty for the next pump.
    pub(crate) fn process_messages(
        &mut self,
        messages: impl IntoIterator<Item = P::Message>,
        invalidated: &AtomicBool,
    ) -> bool {
        let mut messages = messages.into_iter().peekable();
        if messages.peek().is_none() && !invalidated.load(Ordering::Acquire) {
            return false;
        }
        // Run messages against the instance and keep the warm cache for the next rebuild.
        // A synchronous rebuild here would double the per-input build cost.
        for msg in messages {
            let _task = self.tree.instance.update(msg);
        }
        invalidated.store(false, Ordering::Release);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, atomic::AtomicUsize};

    #[derive(Default)]
    struct StubProgram {
        view_calls: Arc<AtomicUsize>,
        updates: Arc<std::sync::Mutex<Vec<u8>>>,
        redraw: Option<(std::time::Instant, bool)>,
    }
    #[derive(Debug, Clone, Default)]
    struct StubState;
    #[derive(Debug, Clone)]
    enum StubMessage {
        Ping(u8),
    }

    impl Program for StubProgram {
        type State = StubState;
        type Message = StubMessage;
        type Theme = iced::Theme;
        type Renderer = Renderer;
        type Executor = iced_winit::futures::backend::default::Executor;

        fn name() -> &'static str {
            "stub"
        }

        fn settings(&self) -> iced::Settings {
            iced::Settings::default()
        }

        fn window(&self) -> Option<iced::window::Settings> {
            None
        }

        fn boot(&self) -> (Self::State, iced::Task<Self::Message>) {
            (StubState, iced::Task::none())
        }

        fn update(
            &self,
            _state: &mut Self::State,
            message: Self::Message,
        ) -> iced::Task<Self::Message> {
            let StubMessage::Ping(value) = message;
            self.updates.lock().unwrap().push(value);
            iced::Task::none()
        }

        fn view<'a>(
            &self,
            _state: &'a Self::State,
            _window: iced::window::Id,
        ) -> iced::Element<'a, Self::Message, Self::Theme, Self::Renderer> {
            self.view_calls.fetch_add(1, Ordering::SeqCst);
            if let Some((deadline, rebuild)) = self.redraw {
                return iced::Element::new(RedrawWidget { deadline, rebuild });
            }
            iced::widget::text("stub").into()
        }
    }

    struct RedrawWidget {
        deadline: std::time::Instant,
        rebuild: bool,
    }

    impl iced::advanced::Widget<StubMessage, iced::Theme, Renderer> for RedrawWidget {
        fn size(&self) -> iced::Size<iced::Length> {
            iced::Size::new(iced::Length::Fill, iced::Length::Fill)
        }

        fn layout(
            &mut self,
            _: &mut iced::advanced::widget::Tree,
            _: &Renderer,
            limits: &iced::advanced::layout::Limits,
        ) -> iced::advanced::layout::Node {
            iced::advanced::layout::Node::new(limits.max())
        }

        fn draw(
            &self,
            _: &iced::advanced::widget::Tree,
            _: &mut Renderer,
            _: &iced::Theme,
            _: &iced::advanced::renderer::Style,
            _: iced::advanced::Layout<'_>,
            _: mouse::Cursor,
            _: &iced::Rectangle,
        ) {
        }

        fn update(
            &mut self,
            _: &mut iced::advanced::widget::Tree,
            event: &iced::Event,
            _: iced::advanced::Layout<'_>,
            _: mouse::Cursor,
            _: &Renderer,
            _: &mut dyn iced::advanced::Clipboard,
            shell: &mut iced::advanced::Shell<'_, StubMessage>,
            _: &iced::Rectangle,
        ) {
            if matches!(
                event,
                iced::Event::Window(iced::window::Event::RedrawRequested(_))
            ) {
                shell.publish(StubMessage::Ping(9));
                shell.request_redraw_at(self.deadline);
                if self.rebuild {
                    shell.invalidate_widgets();
                }
            }
        }
    }

    #[test]
    fn redraw_widget_outputs_messages_deadline_and_rebuild_through_real_ui() {
        for rebuild in [false, true] {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(1);
            let program = StubProgram {
                redraw: Some((deadline, rebuild)),
                ..Default::default()
            };
            let updates = program.updates.clone();
            let (instance, _) = program::Instance::new(program);
            let mut shell = UiShell::new(
                instance,
                iced::window::Id::unique(),
                iced::Size::new(100.0, 100.0),
            );
            let mut renderer = Renderer::new(crate::convert::default_font(), iced::Pixels(16.0));
            let mut clipboard = Clipboard::unconnected();
            let mut messages = Vec::new();
            let outcome = shell.pump(
                iced::Event::Window(iced::window::Event::RedrawRequested(
                    std::time::Instant::now(),
                )),
                mouse::Cursor::default(),
                &mut clipboard,
                &mut renderer,
                &mut messages,
                Some((
                    &iced::Theme::Light,
                    &iced::advanced::renderer::Style::default(),
                )),
            );
            if rebuild {
                assert_eq!(outcome, PumpOutcome::Rebuild);
            } else {
                assert_eq!(outcome, PumpOutcome::At(deadline));
                assert_eq!(shell.deadline, Some(deadline));
            }
            assert!(matches!(messages.as_slice(), [StubMessage::Ping(9)]));
            // Messages apply to the instance now; the view rebuilds on the next pump.
            assert!(shell.process_messages(messages, &AtomicBool::new(rebuild)));
            assert_eq!(*updates.lock().unwrap(), [9]);
            if !rebuild {
                assert_eq!(
                    shell.pump(
                        iced::Event::Window(iced::window::Event::Focused),
                        mouse::Cursor::default(),
                        &mut clipboard,
                        &mut renderer,
                        &mut Vec::new(),
                        None,
                    ),
                    PumpOutcome::Idle
                );
                assert_eq!(
                    shell.deadline,
                    Some(deadline),
                    "unrelated Wait must preserve the timer"
                );
            }
        }
    }

    type StubShell = (
        UiShell<StubProgram>,
        Renderer,
        iced_winit::Clipboard,
        Arc<AtomicUsize>,
        Arc<std::sync::Mutex<Vec<u8>>>,
    );

    fn stub_shell() -> StubShell {
        let program = StubProgram::default();
        let calls = Arc::clone(&program.view_calls);
        let updates = Arc::clone(&program.updates);
        let (instance, _task) = program::Instance::new(program);
        let backend = Renderer::new(crate::convert::default_font(), iced::Pixels(16.0));
        let shell = UiShell::new(
            instance,
            iced::window::Id::unique(),
            iced::Size::new(800.0, 600.0),
        );
        let clipboard = iced_winit::Clipboard::unconnected();
        (shell, backend, clipboard, calls, updates)
    }

    #[test]
    fn pump_outcome_routes_widget_requests() {
        use iced_winit::program::core::InputMethod;

        let updated = |redraw_request| State::Updated {
            mouse_interaction: mouse::Interaction::Idle,
            redraw_request,
            input_method: InputMethod::Disabled,
            has_layout_changed: false,
        };
        // A widget asking for a view rebuild (rare) routes to one.
        assert_eq!(pump_outcome(&State::Outdated), PumpOutcome::Rebuild);
        // Message-less repaint requests (wheel/drag/hover/caret) must
        // repaint: dropping them froze every scrollable.
        assert_eq!(
            pump_outcome(&updated(RedrawRequest::Wait)),
            PumpOutcome::Idle
        );
        assert_eq!(
            pump_outcome(&updated(RedrawRequest::NextFrame)),
            PumpOutcome::Repaint
        );
    }

    #[test]
    fn pump_builds_transient_ui_and_idles() {
        // No retained interface: every pump rebuilds from the cache
        // and folds back, so there is no orphan state to warn on.
        let (mut shell, mut backend, mut clipboard, _, _) = stub_shell();
        let mut messages = Vec::new();
        let outcome = shell.pump(
            iced::Event::Window(iced::window::Event::RedrawRequested(
                std::time::Instant::now(),
            )),
            iced::mouse::Cursor::default(),
            &mut clipboard,
            &mut backend,
            &mut messages,
            None,
        );
        assert!(messages.is_empty());
        assert_eq!(outcome, PumpOutcome::Idle);
        // Relayout only records bounds.
        shell.relayout(iced::Size::new(640.0, 480.0));
        assert_eq!(shell.tree.bounds, iced::Size::new(640.0, 480.0));
        // SPIKE NER-335: client offset is recorded and survives a pump.
        shell.set_content_offset(Some((26.0, 60.0)));
        let outcome = shell.pump(
            iced::Event::Window(iced::window::Event::RedrawRequested(
                std::time::Instant::now(),
            )),
            iced::mouse::Cursor::default(),
            &mut clipboard,
            &mut backend,
            &mut messages,
            None,
        );
        assert_eq!(outcome, PumpOutcome::Idle);
        assert_eq!(shell.tree.offset, Some((26.0, 60.0)));
    }

    #[test]
    fn process_messages_runs_iterator_in_order_and_defers_rebuild() {
        let (mut shell, mut backend, mut clipboard, calls, updates) = stub_shell();
        let invalidated = AtomicBool::new(false);
        assert!(shell.process_messages((0..3).map(StubMessage::Ping), &invalidated));
        assert_eq!(*updates.lock().unwrap(), [0, 1, 2]);
        // No synchronous rebuild: the view builds once on the next pump.
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        assert!(!shell.process_messages(std::iter::empty(), &invalidated));
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        shell.pump(
            iced::Event::Window(iced::window::Event::RedrawRequested(
                std::time::Instant::now(),
            )),
            mouse::Cursor::default(),
            &mut clipboard,
            &mut backend,
            &mut Vec::new(),
            None,
        );
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn process_messages_skips_quiet_batches() {
        let (mut shell, mut backend, mut clipboard, calls, updates) = stub_shell();
        // Transient UI: construction builds nothing; the next pump after
        // a dirty batch invokes the view exactly once.
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        // Empty batch without invalidation: no dirt.
        let still = AtomicBool::new(false);
        assert!(!shell.process_messages(Vec::new(), &still));
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        // Either a message or the flag dirties exactly once.
        assert!(shell.process_messages([StubMessage::Ping(7)], &still));
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        assert_eq!(*updates.lock().unwrap(), [7]);
        let flagged = AtomicBool::new(true);
        assert!(shell.process_messages(Vec::new(), &flagged));
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        assert!(!flagged.load(Ordering::Acquire));
        shell.pump(
            iced::Event::Window(iced::window::Event::RedrawRequested(
                std::time::Instant::now(),
            )),
            mouse::Cursor::default(),
            &mut clipboard,
            &mut backend,
            &mut Vec::new(),
            None,
        );
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    /// Address-field typing survives the message batch between keystrokes with focus intact.
    #[test]
    fn text_input_keeps_focus_across_message_batches() {
        use iced::keyboard::{self, key::Code as KeyCode, key::Named, key::Physical};

        #[derive(Default)]
        struct InputState {
            value: String,
            // Unrelated model field, like a debug drain sync landing in the
            // same batch: it must not disturb the focused input.
            sync_generation: u32,
        }
        #[derive(Debug, Clone)]
        enum InputMessage {
            Typed(String),
            SyncTick(u32),
        }
        #[derive(Default)]
        struct InputProgram;
        impl Program for InputProgram {
            type State = InputState;
            type Message = InputMessage;
            type Theme = iced::Theme;
            type Renderer = Renderer;
            type Executor = iced_winit::futures::backend::default::Executor;

            fn name() -> &'static str {
                "input-stub"
            }

            fn settings(&self) -> iced::Settings {
                iced::Settings::default()
            }

            fn window(&self) -> Option<iced::window::Settings> {
                None
            }

            fn boot(&self) -> (Self::State, iced::Task<Self::Message>) {
                (InputState::default(), iced::Task::none())
            }

            fn update(
                &self,
                state: &mut Self::State,
                message: Self::Message,
            ) -> iced::Task<Self::Message> {
                match message {
                    InputMessage::Typed(value) => state.value = value,
                    InputMessage::SyncTick(generation) => state.sync_generation = generation,
                }
                iced::Task::none()
            }

            fn view<'a>(
                &self,
                state: &'a Self::State,
                _window: iced::window::Id,
            ) -> iced::Element<'a, Self::Message, Self::Theme, Self::Renderer> {
                iced::widget::text_input("addr", &state.value)
                    .on_input(InputMessage::Typed)
                    .into()
            }
        }

        fn redraw() -> iced::Event {
            iced::Event::Window(iced::window::Event::RedrawRequested(
                std::time::Instant::now(),
            ))
        }
        fn press(ch: &str) -> iced::Event {
            iced::Event::Keyboard(keyboard::Event::KeyPressed {
                key: keyboard::Key::Character(ch.into()),
                modified_key: keyboard::Key::Character(ch.into()),
                physical_key: Physical::Code(KeyCode::KeyA),
                location: keyboard::Location::Standard,
                modifiers: keyboard::Modifiers::empty(),
                text: Some(ch.into()),
                repeat: false,
            })
        }
        fn press_left() -> iced::Event {
            iced::Event::Keyboard(keyboard::Event::KeyPressed {
                key: keyboard::Key::Named(Named::ArrowLeft),
                modified_key: keyboard::Key::Named(Named::ArrowLeft),
                physical_key: Physical::Code(KeyCode::ArrowLeft),
                location: keyboard::Location::Standard,
                modifiers: keyboard::Modifiers::empty(),
                text: None,
                repeat: false,
            })
        }
        fn scroll() -> iced::Event {
            iced::Event::Mouse(iced::mouse::Event::WheelScrolled {
                delta: iced::mouse::ScrollDelta::Lines { x: 0.0, y: -1.0 },
            })
        }

        let (instance, _) = program::Instance::new(InputProgram);
        let mut shell = UiShell::new(
            instance,
            iced::window::Id::unique(),
            iced::Size::new(800.0, 600.0),
        );
        let mut backend = Renderer::new(crate::convert::default_font(), iced::Pixels(16.0));
        let mut clipboard = Clipboard::unconnected();
        let mut out = Vec::new();
        let over_input = mouse::Cursor::Available(iced::Point::new(400.0, 10.0));
        // Prime the retained tree, then focus the input with a click.
        shell.pump(
            redraw(),
            mouse::Cursor::default(),
            &mut clipboard,
            &mut backend,
            &mut out,
            None,
        );
        shell.pump(
            iced::Event::Mouse(iced::mouse::Event::ButtonPressed(iced::mouse::Button::Left)),
            over_input,
            &mut clipboard,
            &mut backend,
            &mut out,
            None,
        );
        // First keystroke: the focused widget emits the new value.
        shell.pump(
            press("a"),
            over_input,
            &mut clipboard,
            &mut backend,
            &mut out,
            None,
        );
        assert!(
            matches!(out.as_slice(), [InputMessage::Typed(value)] if value == "a"),
            "first keystroke reaches the focused input: {out:?}"
        );
        // Apply to the model without rebuilding, alongside an unrelated
        // sync update in the same batch: focus and cursor survive both.
        out.push(InputMessage::SyncTick(1));
        assert!(shell.process_messages(std::mem::take(&mut out), &AtomicBool::new(false)));
        // Second keystroke lands in the same input, after the kept cursor.
        shell.pump(
            press("b"),
            over_input,
            &mut clipboard,
            &mut backend,
            &mut out,
            None,
        );
        assert!(
            matches!(out.as_slice(), [InputMessage::Typed(value)] if value == "ab"),
            "second keystroke continues without refocus: {out:?}"
        );
        assert!(shell.process_messages(std::mem::take(&mut out), &AtomicBool::new(false)));
        // Mid-cursor edit: Left moves the kept cursor, so the next keystroke
        // inserts instead of appending. A rebuilt widget would yield "abX".
        shell.pump(
            press_left(),
            over_input,
            &mut clipboard,
            &mut backend,
            &mut out,
            None,
        );
        assert!(out.is_empty(), "cursor move emits no message: {out:?}");
        // A scroll event in between must not disturb the retained input.
        shell.pump(
            scroll(),
            over_input,
            &mut clipboard,
            &mut backend,
            &mut out,
            None,
        );
        shell.pump(
            press("X"),
            over_input,
            &mut clipboard,
            &mut backend,
            &mut out,
            None,
        );
        assert!(
            matches!(out.as_slice(), [InputMessage::Typed(value)] if value == "aXb"),
            "insert lands at the kept cursor: {out:?}"
        );
    }

    /// Text selections survive message batches; the next keystroke overwrites the range.
    #[test]
    fn text_input_selection_survives_message_batches() {
        use iced::keyboard::{self, key::Code as KeyCode, key::Named, key::Physical};

        #[derive(Default)]
        struct InputState {
            value: String,
            sync_generation: u32,
        }
        #[derive(Debug, Clone)]
        enum InputMessage {
            Typed(String),
            SyncTick(u32),
        }
        #[derive(Default)]
        struct InputProgram;
        impl Program for InputProgram {
            type State = InputState;
            type Message = InputMessage;
            type Theme = iced::Theme;
            type Renderer = Renderer;
            type Executor = iced_winit::futures::backend::default::Executor;

            fn name() -> &'static str {
                "input-stub"
            }

            fn settings(&self) -> iced::Settings {
                iced::Settings::default()
            }

            fn window(&self) -> Option<iced::window::Settings> {
                None
            }

            fn boot(&self) -> (Self::State, iced::Task<Self::Message>) {
                (InputState::default(), iced::Task::none())
            }

            fn update(
                &self,
                state: &mut Self::State,
                message: Self::Message,
            ) -> iced::Task<Self::Message> {
                match message {
                    InputMessage::Typed(value) => state.value = value,
                    InputMessage::SyncTick(generation) => state.sync_generation = generation,
                }
                iced::Task::none()
            }

            fn view<'a>(
                &self,
                state: &'a Self::State,
                _window: iced::window::Id,
            ) -> iced::Element<'a, Self::Message, Self::Theme, Self::Renderer> {
                iced::widget::text_input("addr", &state.value)
                    .on_input(InputMessage::Typed)
                    .into()
            }
        }

        fn redraw() -> iced::Event {
            iced::Event::Window(iced::window::Event::RedrawRequested(
                std::time::Instant::now(),
            ))
        }
        fn press(ch: &str) -> iced::Event {
            iced::Event::Keyboard(keyboard::Event::KeyPressed {
                key: keyboard::Key::Character(ch.into()),
                modified_key: keyboard::Key::Character(ch.into()),
                physical_key: Physical::Code(KeyCode::KeyA),
                location: keyboard::Location::Standard,
                modifiers: keyboard::Modifiers::empty(),
                text: Some(ch.into()),
                repeat: false,
            })
        }
        fn shift_left() -> iced::Event {
            iced::Event::Keyboard(keyboard::Event::KeyPressed {
                key: keyboard::Key::Named(Named::ArrowLeft),
                modified_key: keyboard::Key::Named(Named::ArrowLeft),
                physical_key: Physical::Code(KeyCode::ArrowLeft),
                location: keyboard::Location::Standard,
                modifiers: keyboard::Modifiers::empty(),
                text: None,
                repeat: false,
            })
        }
        fn modifiers_changed(modifiers: keyboard::Modifiers) -> iced::Event {
            iced::Event::Keyboard(keyboard::Event::ModifiersChanged(modifiers))
        }
        fn select_all() -> iced::Event {
            iced::Event::Keyboard(keyboard::Event::KeyPressed {
                key: keyboard::Key::Character("a".into()),
                modified_key: keyboard::Key::Character("a".into()),
                physical_key: Physical::Code(KeyCode::KeyA),
                location: keyboard::Location::Standard,
                modifiers: keyboard::Modifiers::COMMAND,
                text: Some("a".into()),
                repeat: false,
            })
        }

        let (instance, _) = program::Instance::new(InputProgram);
        let mut shell = UiShell::new(
            instance,
            iced::window::Id::unique(),
            iced::Size::new(800.0, 600.0),
        );
        let mut backend = Renderer::new(crate::convert::default_font(), iced::Pixels(16.0));
        let mut clipboard = Clipboard::unconnected();
        let mut out = Vec::new();
        let over_input = mouse::Cursor::Available(iced::Point::new(400.0, 10.0));
        shell.pump(
            redraw(),
            mouse::Cursor::default(),
            &mut clipboard,
            &mut backend,
            &mut out,
            None,
        );
        shell.pump(
            iced::Event::Mouse(iced::mouse::Event::ButtonPressed(iced::mouse::Button::Left)),
            over_input,
            &mut clipboard,
            &mut backend,
            &mut out,
            None,
        );
        for ch in ["a", "b"] {
            shell.pump(
                press(ch),
                over_input,
                &mut clipboard,
                &mut backend,
                &mut out,
                None,
            );
            assert!(shell.process_messages(std::mem::take(&mut out), &AtomicBool::new(false)));
        }
        // Range-select across a batch: the next keystroke overwrites the range.
        shell.pump(
            modifiers_changed(keyboard::Modifiers::SHIFT),
            over_input,
            &mut clipboard,
            &mut backend,
            &mut out,
            None,
        );
        shell.pump(
            shift_left(),
            over_input,
            &mut clipboard,
            &mut backend,
            &mut out,
            None,
        );
        shell.pump(
            modifiers_changed(keyboard::Modifiers::empty()),
            over_input,
            &mut clipboard,
            &mut backend,
            &mut out,
            None,
        );
        assert!(out.is_empty(), "range select emits no message: {out:?}");
        shell.pump(
            press("X"),
            over_input,
            &mut clipboard,
            &mut backend,
            &mut out,
            None,
        );
        assert!(
            matches!(out.as_slice(), [InputMessage::Typed(value)] if value == "aX"),
            "keystroke overwrites the kept selection: {out:?}"
        );
        // Unrelated sync rides the same batch without disturbing the input.
        out.push(InputMessage::SyncTick(1));
        assert!(shell.process_messages(std::mem::take(&mut out), &AtomicBool::new(false)));
        // Select-all across a batch: the next keystroke replaces all.
        shell.pump(
            modifiers_changed(keyboard::Modifiers::COMMAND),
            over_input,
            &mut clipboard,
            &mut backend,
            &mut out,
            None,
        );
        shell.pump(
            select_all(),
            over_input,
            &mut clipboard,
            &mut backend,
            &mut out,
            None,
        );
        shell.pump(
            modifiers_changed(keyboard::Modifiers::empty()),
            over_input,
            &mut clipboard,
            &mut backend,
            &mut out,
            None,
        );
        assert!(out.is_empty(), "select-all emits no message: {out:?}");
        shell.pump(
            press("Y"),
            over_input,
            &mut clipboard,
            &mut backend,
            &mut out,
            None,
        );
        assert!(
            matches!(out.as_slice(), [InputMessage::Typed(value)] if value == "Y"),
            "keystroke replaces the select-all range: {out:?}"
        );
    }

    /// A scrollable keeps its offset across message batches.
    #[test]
    fn scrollable_offset_survives_message_batches() {
        #[derive(Default)]
        struct ScrollState {
            ticks: u32,
        }
        #[derive(Debug, Clone)]
        enum ScrollMessage {
            Scrolled(f32),
            SyncTick(u32),
        }
        #[derive(Default)]
        struct ScrollProgram;
        impl Program for ScrollProgram {
            type State = ScrollState;
            type Message = ScrollMessage;
            type Theme = iced::Theme;
            type Renderer = Renderer;
            type Executor = iced_winit::futures::backend::default::Executor;

            fn name() -> &'static str {
                "scroll-stub"
            }

            fn settings(&self) -> iced::Settings {
                iced::Settings::default()
            }

            fn window(&self) -> Option<iced::window::Settings> {
                None
            }

            fn boot(&self) -> (Self::State, iced::Task<Self::Message>) {
                (ScrollState::default(), iced::Task::none())
            }

            fn update(
                &self,
                state: &mut Self::State,
                message: Self::Message,
            ) -> iced::Task<Self::Message> {
                if let ScrollMessage::SyncTick(ticks) = message {
                    state.ticks = ticks;
                }
                iced::Task::none()
            }

            fn view<'a>(
                &self,
                _state: &'a Self::State,
                _window: iced::window::Id,
            ) -> iced::Element<'a, Self::Message, Self::Theme, Self::Renderer> {
                let content = (0..50).fold(iced::widget::Column::new(), |column, i| {
                    column.push(iced::widget::text(format!("row {i}")))
                });
                iced::widget::column![
                    iced::widget::scrollable(content)
                        .width(iced::Length::Fill)
                        .height(iced::Length::Fixed(120.0))
                        .on_scroll(|viewport| ScrollMessage::Scrolled(
                            viewport.absolute_offset().y
                        )),
                    iced::widget::text("probe"),
                ]
                .into()
            }
        }

        fn redraw() -> iced::Event {
            iced::Event::Window(iced::window::Event::RedrawRequested(
                std::time::Instant::now(),
            ))
        }
        fn wheel() -> iced::Event {
            iced::Event::Mouse(iced::mouse::Event::WheelScrolled {
                delta: iced::mouse::ScrollDelta::Lines { x: 0.0, y: -1.0 },
            })
        }

        let (instance, _) = program::Instance::new(ScrollProgram);
        let mut shell = UiShell::new(
            instance,
            iced::window::Id::unique(),
            iced::Size::new(800.0, 600.0),
        );
        let mut backend = Renderer::new(crate::convert::default_font(), iced::Pixels(16.0));
        let mut clipboard = Clipboard::unconnected();
        let mut out = Vec::new();
        let over_list = mouse::Cursor::Available(iced::Point::new(400.0, 10.0));
        shell.pump(
            redraw(),
            mouse::Cursor::default(),
            &mut clipboard,
            &mut backend,
            &mut out,
            None,
        );
        shell.pump(
            wheel(),
            over_list,
            &mut clipboard,
            &mut backend,
            &mut out,
            None,
        );
        let first = match out.as_slice() {
            [ScrollMessage::Scrolled(y)] => *y,
            other => panic!("first scroll reports its offset: {other:?}"),
        };
        // Unrelated sync in the same batch must not reset the offset.
        out.push(ScrollMessage::SyncTick(1));
        assert!(shell.process_messages(std::mem::take(&mut out), &AtomicBool::new(false)));
        shell.pump(
            wheel(),
            over_list,
            &mut clipboard,
            &mut backend,
            &mut out,
            None,
        );
        let second = match out.as_slice() {
            [ScrollMessage::Scrolled(y)] => *y,
            other => panic!("second scroll reports its offset: {other:?}"),
        };
        assert!(
            second > first,
            "offset accumulates on the retained tree ({first} -> {second})"
        );
    }
}
