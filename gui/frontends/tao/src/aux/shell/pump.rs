//! Program pump: retained tree plus queued messages, no live interface.
//! Relayout, pump, and message-run passes live here; surfaces live in `surface`.

use std::sync::atomic::{AtomicBool, Ordering};

use iced::{Size, mouse};
use iced_tiny_skia::Renderer;
use iced_winit::{
    Clipboard,
    program::{self, Program, core::window::RedrawRequest},
    runtime::user_interface::{Cache, State, UserInterface},
};

/// Program UI shell: [`ViewTree`] plus a [`MessageQueue`] on the side.
/// No interface is retained across calls.
pub(crate) struct ViewTree<P: Program<Theme = iced::Theme, Renderer = Renderer>> {
    pub(super) instance: Box<program::Instance<P>>,
    pub(super) cache: Cache,
    pub(super) window_id: iced::window::Id,
    pub(super) bounds: Size,
}

/// Queued program messages, drained under the update-then-rebuild sequence.
pub(crate) struct MessageQueue<M> {
    pending: Vec<M>,
}

impl<M> MessageQueue<M> {
    fn new() -> Self {
        Self {
            pending: Vec::new(),
        }
    }

    /// Queued plus inline messages for one rebuild pass.
    fn drain(&mut self, extra: impl IntoIterator<Item = M>) -> Vec<M> {
        let mut all = std::mem::take(&mut self.pending);
        all.extend(extra);
        all
    }
}

/// Program UI shell: [`ViewTree`] plus a [`MessageQueue`] on the side.
/// No interface is retained across calls.
pub(crate) struct UiShell<P: Program<Theme = iced::Theme, Renderer = Renderer>> {
    pub(super) tree: ViewTree<P>,
    queue: MessageQueue<P::Message>,
    /// Reused event-draw scratch: pumps and redraws never allocate per event.
    scratch: Vec<P::Message>,
}

/// What a pumped event needs after `update`: nothing, a repaint,
/// or a view rebuild (`State::Outdated`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PumpOutcome {
    Idle,
    Repaint,
    Rebuild,
}

/// Pump policy: map iced's update result onto [`PumpOutcome`]. Pure, so tests cover it.
pub(crate) fn pump_outcome(state: &State) -> PumpOutcome {
    match state {
        State::Outdated => PumpOutcome::Rebuild,
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
            },
            queue: MessageQueue::new(),
            scratch: Vec::new(),
        }
    }

    /// Record new layout bounds. Infallible: the next pump builds against them.
    pub(crate) fn relayout(&mut self, bounds: Size) {
        self.tree.bounds = bounds;
    }

    /// Record new layout bounds via the shell (single mutator: no
    /// direct `tree.bounds` writes outside this impl).
    pub(crate) fn set_bounds(&mut self, bounds: Size) {
        self.relayout(bounds);
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

    /// Queue one program message for the next `process_messages` pass.
    /// Queued, never applied inline: only `process_messages` may mutate the instance.
    #[cfg(test)]
    pub(crate) fn push_message(&mut self, message: P::Message) {
        self.queue.pending.push(message);
    }

    /// Build the transient interface for one pump pass.
    /// The returned interface borrows the instance; callers pass disjoint tree fields.
    fn build_transient<'a>(
        instance: &'a program::Instance<P>,
        window_id: iced::window::Id,
        bounds: Size,
        cache: &mut Cache,
        renderer: &mut Renderer,
    ) -> UserInterface<'a, P::Message, iced::Theme, Renderer> {
        let cache = std::mem::take(cache);
        UserInterface::build(instance.view(window_id), bounds, cache, renderer)
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
    ) -> PumpOutcome {
        out.clear();
        // Disjoint tree fields: the interface borrows the instance
        // while the cache slot is taken out and folded back.
        let mut ui = Self::build_transient(
            &self.tree.instance,
            self.tree.window_id,
            self.tree.bounds,
            &mut self.tree.cache,
            backend,
        );
        let (state, _) = ui.update(&[mapped], cursor, backend, clipboard, out);
        let outcome = pump_outcome(&state);
        Self::retire_transient(&mut self.tree.cache, ui);
        outcome
    }

    /// Runs pending messages, rebuilding once. Reports whether a rebuild happened.
    pub(crate) fn process_messages(
        &mut self,
        messages: impl IntoIterator<Item = P::Message>,
        renderer: &mut Renderer,
        invalidated: &AtomicBool,
    ) -> bool {
        // Queued `push_message` items join this pass so the single
        // build below applies every pending program message at once.
        let mut all = self.queue.drain(messages);
        if all.is_empty() && !invalidated.load(Ordering::Acquire) {
            return false;
        }
        // Run the messages against the instance, then rebuild once.
        for msg in all.drain(..) {
            let _task = self.tree.instance.update(msg);
        }
        let ui = Self::build_transient(
            &self.tree.instance,
            self.tree.window_id,
            self.tree.bounds,
            &mut self.tree.cache,
            renderer,
        );
        Self::retire_transient(&mut self.tree.cache, ui);
        invalidated.store(false, Ordering::Release);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    static VIEW_CALLS: AtomicUsize = AtomicUsize::new(0);

    struct StubProgram;
    #[derive(Debug, Clone, Default)]
    struct StubState;
    #[derive(Debug, Clone)]
    enum StubMessage {
        Ping,
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
            _message: Self::Message,
        ) -> iced::Task<Self::Message> {
            iced::Task::none()
        }

        fn view<'a>(
            &self,
            _state: &'a Self::State,
            _window: iced::window::Id,
        ) -> iced::Element<'a, Self::Message, Self::Theme, Self::Renderer> {
            VIEW_CALLS.fetch_add(1, Ordering::SeqCst);
            iced::widget::text("stub").into()
        }
    }

    fn stub_shell() -> (UiShell<StubProgram>, Renderer, iced_winit::Clipboard) {
        VIEW_CALLS.store(0, Ordering::SeqCst);
        let (instance, _task) = program::Instance::new(StubProgram);
        let backend = Renderer::new(crate::convert::default_font(), iced::Pixels(16.0));
        let shell = UiShell::new(
            instance,
            iced::window::Id::unique(),
            iced::Size::new(800.0, 600.0),
        );
        let clipboard = iced_winit::Clipboard::unconnected();
        (shell, backend, clipboard)
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
        let (mut shell, mut backend, mut clipboard) = stub_shell();
        let mut messages = Vec::new();
        let outcome = shell.pump(
            iced::Event::Window(iced::window::Event::RedrawRequested(
                std::time::Instant::now(),
            )),
            iced::mouse::Cursor::default(),
            &mut clipboard,
            &mut backend,
            &mut messages,
        );
        assert!(messages.is_empty());
        assert_eq!(outcome, PumpOutcome::Idle);
        // Push and relayout stay total; relayout only records bounds.
        shell.push_message(StubMessage::Ping);
        shell.relayout(iced::Size::new(640.0, 480.0));
        assert_eq!(shell.tree.bounds, iced::Size::new(640.0, 480.0));
    }

    #[test]
    fn shell_drops_cleanly_without_retained_ui() {
        // No self-reference remains: live and message-queued shells
        // both drop without a manual drop order.
        let (shell, _, _) = stub_shell();
        drop(shell);
        let (mut shell, _, _) = stub_shell();
        shell.push_message(StubMessage::Ping);
        drop(shell);
    }

    #[test]
    fn message_queue_drains_pending_then_extra_in_order() {
        let mut queue = MessageQueue::new();
        assert_eq!(queue.drain([StubMessage::Ping]).len(), 1);
        queue.pending.push(StubMessage::Ping);
        queue.pending.push(StubMessage::Ping);
        // Queued items come first, inline extras appended after.
        assert_eq!(queue.drain([StubMessage::Ping]).len(), 3);
        assert!(queue.drain([] as [StubMessage; 0]).is_empty());
    }

    #[test]
    fn set_bounds_aliases_relayout() {
        let (mut shell, _, _) = stub_shell();
        shell.set_bounds(iced::Size::new(0.0, 0.0));
        assert_eq!(shell.tree.bounds, iced::Size::new(0.0, 0.0));
    }

    #[test]
    fn process_messages_skips_quiet_batches() {
        let (mut shell, mut backend, _) = stub_shell();
        // Transient UI: construction builds nothing; each rebuild
        // invokes the view exactly once.
        assert_eq!(VIEW_CALLS.load(Ordering::SeqCst), 0);
        // Empty batch without invalidation: no rebuild.
        let still = AtomicBool::new(false);
        assert!(!shell.process_messages(Vec::new(), &mut backend, &still));
        assert_eq!(VIEW_CALLS.load(Ordering::SeqCst), 0);
        // Either a message or the flag rebuilds exactly once.
        assert!(shell.process_messages(vec![StubMessage::Ping], &mut backend, &still));
        assert_eq!(VIEW_CALLS.load(Ordering::SeqCst), 1);
        let flagged = AtomicBool::new(true);
        assert!(shell.process_messages(Vec::new(), &mut backend, &flagged));
        assert_eq!(VIEW_CALLS.load(Ordering::SeqCst), 2);
        assert!(!flagged.load(Ordering::Acquire));
    }
}
