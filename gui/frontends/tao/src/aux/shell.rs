//! Shared iced-on-tao auxiliary-window shell: debugger and PPU viewer
//! run the same tiny-skia lifecycle over different programs.

use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use iced::{Size, advanced::renderer, mouse, theme};
use iced_tiny_skia::{
    Renderer,
    graphics::compositor::Compositor as _,
    window::{Compositor, Surface, compositor},
};
use iced_winit::{
    Clipboard,
    graphics::Viewport,
    program::{self, Program, core::window::RedrawRequest},
    runtime::user_interface::{Cache, State, UserInterface},
};
use tao::{
    event_loop::EventLoopWindowTarget,
    window::{Window as TaoWindow, WindowBuilder},
};

#[cfg(target_os = "macos")]
use tao::platform::macos::WindowBuilderExtMacOS;

use crate::{convert::default_font, event::UserEvent};

/// Light theme for OS light-theme nativeness. Every auxiliary
/// program shares it (the flagged duplication was five copies).
pub(crate) fn aux_theme() -> Option<iced::Theme> {
    Some(iced::Theme::Light)
}

/// Shared program settings: one default font and text size.
pub(crate) fn aux_settings() -> iced::Settings {
    iced::Settings {
        default_font: default_font(),
        default_text_size: iced::Pixels(16.0),
        ..Default::default()
    }
}

/// Object-safe window-system surface behind every aux handle.
/// Lets the focus/registry layers route without naming the program type.
pub(crate) trait AuxShellOps {
    fn tao_window_id(&self) -> tao::window::WindowId;
    fn close_requested(&self) -> bool;
    fn focus_window(&self);
    fn request_redraw(&self);
    fn handle_common(&mut self, event: &tao::event::WindowEvent);
    fn feed_event(&mut self, event: tao::event::WindowEvent) -> Option<iced::Event>;
    fn render_tagged(&mut self, tag: &'static str);
}

/// Window creation plus the live window system in one surface.
/// Programs and UI instances stay per-window in [`UiShell`].
pub(crate) struct AuxSurface {
    pub(crate) window: Arc<TaoWindow>,
    pub(crate) window_id: iced::window::Id,
    pub(crate) scale_factor: f32,
    pub(crate) viewport_physical: (u32, u32),
    pub(crate) bounds: Size,
    compositor: Compositor,
    surface: Surface,
    backend: Renderer,
    /// Last surface-configured size. Resize storms only relayout;
    /// the realloc happens in `render`, once per presented frame.
    surface_size: (u32, u32),
    modifiers: iced::keyboard::Modifiers,
    should_close: Arc<AtomicBool>,
    cursor: mouse::Cursor,
    clipboard: Clipboard,
}

/// Why an aux window failed to open: which window plus the native error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ScaffoldError {
    BuildWindow { tag: &'static str, detail: String },
}

impl core::fmt::Display for ScaffoldError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            ScaffoldError::BuildWindow { tag, detail } => {
                write!(f, "native window build failed for {tag}: {detail}")
            }
        }
    }
}

impl AuxSurface {
    pub(crate) fn open(
        event_loop: &EventLoopWindowTarget<UserEvent>,
        title: &str,
        size: (f64, f64),
        min_size: Option<(f64, f64)>,
        position: Option<(i32, i32)>,
        log_tag: &'static str,
        should_close: Arc<AtomicBool>,
    ) -> Result<Self, ScaffoldError> {
        let mut builder = WindowBuilder::new()
            .with_title(title)
            .with_inner_size(tao::dpi::LogicalSize::new(size.0, size.1));
        if let Some((w, h)) = min_size {
            builder = builder.with_min_inner_size(tao::dpi::LogicalSize::new(w, h));
        }
        #[cfg(target_os = "macos")]
        {
            builder = builder.with_automatic_window_tabbing(false);
        }
        if let Some((x, y)) = position {
            builder = builder.with_position(tao::dpi::LogicalPosition::new(x as f64, y as f64));
        }
        let window = Arc::new(match builder.build(event_loop) {
            Ok(window) => window,
            Err(error) => {
                log::error!("failed to create {log_tag}: {error}");
                return Err(ScaffoldError::BuildWindow {
                    tag: log_tag,
                    detail: error.to_string(),
                });
            }
        });
        let window_id = iced::window::Id::unique();
        let scale_factor = window.scale_factor() as f32;
        let window_size = window.inner_size();
        let viewport_physical = (window_size.width, window_size.height);
        let logical_size = window_size.to_logical::<f64>(scale_factor as f64);
        let bounds = Size::new(logical_size.width as f32, logical_size.height as f32);
        let mut compositor = compositor::new(
            iced_tiny_skia::Settings {
                default_font: default_font(),
                default_text_size: iced::Pixels(16.0),
            },
            Arc::clone(&window),
        );
        let renderer = compositor.create_renderer();
        let surface =
            compositor.create_surface(Arc::clone(&window), window_size.width, window_size.height);
        Ok(Self {
            window,
            window_id,
            scale_factor,
            viewport_physical,
            bounds,
            compositor,
            backend: renderer,
            surface,
            surface_size: viewport_physical,
            modifiers: iced::keyboard::Modifiers::default(),
            should_close,
            cursor: mouse::Cursor::default(),
            clipboard: Clipboard::unconnected(),
        })
    }

    /// Native window identity for event routing.
    fn tao_window_id(&self) -> tao::window::WindowId {
        self.window.id()
    }

    /// Close was requested through the shared flag.
    fn close_requested(&self) -> bool {
        self.should_close.load(std::sync::atomic::Ordering::Acquire)
    }

    /// Native redraw request without reaching into the window.
    fn request_redraw(&self) {
        self.window.request_redraw();
    }

    /// Bring this window to the front.
    fn focus_window(&self) {
        self.window.set_focus();
    }

    /// Resize the raster and record fresh bounds (no view rebuild).
    /// The surface realloc is deferred to `render`; same-size events skip everything.
    fn resize(&mut self, width: u32, height: u32, relayout: &mut impl FnMut(Size)) {
        if self.viewport_physical == (width, height) {
            return;
        }
        self.viewport_physical = (width, height);
        log::debug!(
            "aux resize: physical={width}x{height} scale={} bounds={:?}",
            self.scale_factor,
            logical_bounds((width, height), self.scale_factor)
        );
        relayout(logical_bounds((width, height), self.scale_factor));
        self.window.request_redraw();
    }

    /// Scale changes move every layout bound: record like resize.
    fn set_scale_factor(&mut self, scale_factor: f32, relayout: &mut impl FnMut(Size)) {
        self.scale_factor = scale_factor;
        relayout(logical_bounds(self.viewport_physical, self.scale_factor));
        self.window.request_redraw();
    }

    fn set_modifiers(&mut self, modifiers: tao::keyboard::ModifiersState) {
        self.modifiers = crate::convert::tao_modifiers_to_iced(modifiers);
    }

    /// Shared event prelude (resize/scale/modifiers/close).
    fn handle_common_event(
        &mut self,
        event: &tao::event::WindowEvent,
        relayout: &mut impl FnMut(Size),
    ) {
        match event {
            tao::event::WindowEvent::Resized(size) => {
                self.resize(size.width, size.height, relayout)
            }
            tao::event::WindowEvent::ScaleFactorChanged { scale_factor, .. } => {
                self.set_scale_factor(*scale_factor as f32, relayout);
            }
            tao::event::WindowEvent::ModifiersChanged(state) => self.set_modifiers(*state),
            tao::event::WindowEvent::CloseRequested => {
                self.should_close
                    .store(true, std::sync::atomic::Ordering::Release);
            }
            _ => {}
        }
    }

    /// Feed one native event, returning the converted iced event.
    fn feed_tao_event(&mut self, event: tao::event::WindowEvent) -> Option<iced::Event> {
        crate::convert::convert_tao_window_event(
            event,
            &mut self.cursor,
            self.scale_factor,
            &mut self.modifiers,
            &self.should_close,
        )
    }

    /// Logical layout bounds for the current viewport and scale.
    fn logical_bounds(&self) -> Size {
        logical_bounds(self.viewport_physical, self.scale_factor)
    }

    /// Surface realloc, at most once per frame.
    fn configure_surface(&mut self) {
        if self.surface_size != self.viewport_physical {
            let (width, height) = self.viewport_physical;
            if width != 0 && height != 0 {
                self.compositor
                    .configure_surface(&mut self.surface, width, height);
            }
            self.surface_size = self.viewport_physical;
        }
    }

    /// Self-healing scale: re-query the live factor and relayout on drift.
    fn heal_scale(&mut self, log_tag: &'static str, relayout: &mut impl FnMut(Size)) {
        let live_scale = self.window.scale_factor() as f32;
        if live_scale != self.scale_factor {
            log::debug!(
                "{log_tag} render: scale drift {} -> {}; relayout",
                self.scale_factor,
                live_scale
            );
            self.scale_factor = live_scale;
            relayout(logical_bounds(self.viewport_physical, self.scale_factor));
        }
    }

    fn present(
        &mut self,
        viewport: &Viewport,
        background_color: iced::Color,
        log_tag: &'static str,
    ) {
        if let Err(error) = self.compositor.present(
            &mut self.backend,
            &mut self.surface,
            viewport,
            background_color,
            || {},
        ) {
            log::warn!("{log_tag} render present failed: {error}");
        }
    }
}

/// Logical layout bounds for a physical viewport and scale factor.
pub(crate) fn logical_bounds(viewport_physical: (u32, u32), scale_factor: f32) -> Size {
    Viewport::with_physical_size(
        Size::new(viewport_physical.0, viewport_physical.1),
        scale_factor,
    )
    .logical_size()
}
/// Program UI shell: [`ViewTree`] plus a [`MessageQueue`] on the side.
/// No interface is retained across calls.
pub(crate) struct ViewTree<P: Program<Theme = iced::Theme, Renderer = Renderer>> {
    instance: Box<program::Instance<P>>,
    cache: Cache,
    window_id: iced::window::Id,
    bounds: Size,
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
    tree: ViewTree<P>,
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

/// One window-system surface plus one program shell.
/// Program-specific halves stay in the handles.
pub(crate) struct AuxShell<P: Program<Theme = iced::Theme, Renderer = Renderer>> {
    surface: AuxSurface,
    ui_state: UiShell<P>,
}

impl<P: Program<Theme = iced::Theme, Renderer = Renderer>> AuxShellOps for AuxShell<P> {
    fn tao_window_id(&self) -> tao::window::WindowId {
        self.surface.tao_window_id()
    }

    fn close_requested(&self) -> bool {
        self.surface.close_requested()
    }

    fn focus_window(&self) {
        self.surface.focus_window();
    }

    fn request_redraw(&self) {
        self.surface.request_redraw();
    }

    fn handle_common(&mut self, event: &tao::event::WindowEvent) {
        let ui_state = &mut self.ui_state;
        self.surface
            .handle_common_event(event, &mut |bounds| ui_state.relayout(bounds));
    }

    fn feed_event(&mut self, event: tao::event::WindowEvent) -> Option<iced::Event> {
        self.surface.feed_tao_event(event)
    }

    fn render_tagged(&mut self, tag: &'static str) {
        self.render(tag);
    }
}

impl<P: Program<Theme = iced::Theme, Renderer = Renderer>> AuxShell<P> {
    /// Bring this window to the front.
    pub(crate) fn focus_window(&self) {
        self.surface.focus_window();
    }

    /// Pump one converted event through the retained tree.
    /// Reuses `out` to avoid a per-event alloc.
    pub(crate) fn pump_frame(
        &mut self,
        mapped: iced::Event,
        out: &mut Vec<P::Message>,
    ) -> PumpOutcome {
        let outcome = self.ui_state.pump(
            mapped,
            self.surface.cursor,
            &mut self.surface.clipboard,
            &mut self.surface.backend,
            out,
        );
        if matches!(outcome, PumpOutcome::Repaint) {
            self.surface.request_redraw();
        }
        outcome
    }

    /// Pump plus finish through the retained scratch buffer.
    /// One line per aux window; no per-event alloc.
    pub(crate) fn pump_finish(
        &mut self,
        mapped: iced::Event,
        extra: impl IntoIterator<Item = P::Message>,
        invalidated: &AtomicBool,
    ) -> bool {
        let mut out = std::mem::take(&mut self.ui_state.scratch);
        let outcome = self.pump_frame(mapped, &mut out);
        out.extend(extra);
        let done = self.finish(outcome, &mut out, invalidated);
        out.clear();
        self.ui_state.scratch = out;
        done
    }

    /// Shared pump finish for the four aux windows: repaint, rebuild, conditional process.
    pub(crate) fn finish(
        &mut self,
        outcome: PumpOutcome,
        messages: &mut Vec<P::Message>,
        invalidated: &AtomicBool,
    ) -> bool {
        let repainted = matches!(outcome, PumpOutcome::Repaint);
        if matches!(outcome, PumpOutcome::Rebuild) {
            invalidated.store(true, Ordering::Release);
        }
        let rebuilt = if !messages.is_empty() || matches!(outcome, PumpOutcome::Rebuild) {
            self.process(messages.drain(..), invalidated)
        } else {
            false
        };
        repainted || rebuilt
    }

    /// Run pending messages against the retained cache, redrawing only on a real rebuild.
    /// Quiet batches (no messages, no flag) skip the frame.
    pub(crate) fn process(
        &mut self,
        messages: impl IntoIterator<Item = P::Message>,
        invalidated: &AtomicBool,
    ) -> bool {
        let bounds = self.surface.logical_bounds();
        self.ui_state.set_bounds(bounds);
        let rebuilt =
            self.ui_state
                .process_messages(messages, &mut self.surface.backend, invalidated);
        if rebuilt {
            self.surface.request_redraw();
        }
        rebuilt
    }
}

impl<P: Program<Theme = iced::Theme, Renderer = Renderer>> AuxShell<P> {
    /// Assemble from a surface plus a program instance.
    /// The surface already carries the shared close flag.
    pub(crate) fn assemble(surface: AuxSurface, instance: program::Instance<P>) -> Self {
        let (window_id, bounds) = (surface.window_id, surface.bounds);
        surface.request_redraw();
        Self {
            surface,
            ui_state: UiShell::new(instance, window_id, bounds),
        }
    }

    /// Full render through the shared pump; only the log tag differs.
    pub(crate) fn render(&mut self, log_tag: &'static str) {
        let ui_state = &mut self.ui_state;
        self.surface
            .heal_scale(log_tag, &mut |bounds| ui_state.relayout(bounds));
        // Deferred surface configure: the realloc lands here, at most once per frame.
        self.surface.configure_surface();
        self.render_ui(log_tag);
    }

    /// Shared full render: Light theme, redraw pump, draw, present.
    fn render_ui(&mut self, log_tag: &'static str) {
        // Nothing to draw into yet (minimize/dock transient):
        // presenting a zero-size surface panics in pixel-map creation.
        if self.surface.viewport_physical.0 == 0 || self.surface.viewport_physical.1 == 0 {
            return;
        }
        let theme = iced::Theme::Light;
        let style = <iced::Theme as theme::Base>::base(&theme);
        let viewport = Viewport::with_physical_size(
            Size::new(
                self.surface.viewport_physical.0,
                self.surface.viewport_physical.1,
            ),
            self.surface.scale_factor,
        );
        let redraw_event = iced::Event::Window(iced::window::Event::RedrawRequested(
            std::time::Instant::now(),
        ));
        {
            let tree = &mut self.ui_state.tree;
            let cache = std::mem::take(&mut tree.cache);
            let mut sink = std::mem::take(&mut self.ui_state.scratch);
            sink.clear();
            let mut ui = UserInterface::build(
                tree.instance.view(tree.window_id),
                tree.bounds,
                cache,
                &mut self.surface.backend,
            );
            // Redraw-only pump: widget rebuild requests wait for the next
            // event-driven pump (`pump_outcome` mapping lives there).
            let _ = ui.update(
                &[redraw_event],
                self.surface.cursor,
                &mut self.surface.backend,
                &mut self.surface.clipboard,
                &mut sink,
            );
            ui.draw(
                &mut self.surface.backend,
                &theme,
                &renderer::Style {
                    text_color: style.text_color,
                },
                self.surface.cursor,
            );
            tree.cache = ui.into_cache();
            self.ui_state.scratch = sink;
        }
        self.surface
            .present(&viewport, style.background_color, log_tag);
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
    fn logical_bounds_divide_by_scale() {
        // Resize/scale rebuilds share this math: 2x shows half the
        // logical pixels of the physical viewport.
        let bounds = logical_bounds((1100, 800), 2.0);
        assert_eq!((bounds.width, bounds.height), (550.0, 400.0));
        let unit = logical_bounds((960, 720), 1.0);
        assert_eq!((unit.width, unit.height), (960.0, 720.0));
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
    fn shared_chrome_helpers_are_stable() {
        assert!(matches!(aux_theme(), Some(iced::Theme::Light)));
        let settings = aux_settings();
        assert_eq!(settings.default_text_size, iced::Pixels(16.0));
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
