//! Shared iced-on-tao auxiliary-window shell: the debugger and PPU
//! viewer run the same tiny-skia lifecycle over different programs,
//! keeping the twins from drifting.

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

/// Window creation scaffolding both auxiliary windows share: title,
/// size, optional minimum, position, compositor, renderer, surface,
/// and initial bounds. Programs and UI instances stay per-window.
pub(crate) struct WindowScaffold {
    pub(crate) window: Arc<TaoWindow>,
    pub(crate) window_id: iced::window::Id,
    pub(crate) scale_factor: f32,
    pub(crate) viewport_physical: (u32, u32),
    pub(crate) bounds: Size,
    pub(crate) compositor: Compositor,
    pub(crate) renderer: Renderer,
    pub(crate) surface: Surface,
}

/// Why an aux window failed to open. Carries which window and the
/// native error so headless/CI failures name the lost window instead
/// of collapsing to a unit variant.
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

impl WindowScaffold {
    pub(crate) fn open(
        event_loop: &EventLoopWindowTarget<UserEvent>,
        title: &str,
        size: (f64, f64),
        min_size: Option<(f64, f64)>,
        position: Option<(i32, i32)>,
        log_tag: &'static str,
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
            renderer,
            surface,
        })
    }
}

/// Logical layout bounds for a physical viewport and scale factor.
/// One helper so every resize/scale path rebuilds with identical
/// math instead of drifting copies.
pub(crate) fn logical_bounds(viewport_physical: (u32, u32), scale_factor: f32) -> Size {
    Viewport::with_physical_size(
        Size::new(viewport_physical.0, viewport_physical.1),
        scale_factor,
    )
    .logical_size()
}
/// Renderer triple both auxiliary windows share. Field names mirror
/// the former per-window structs so call sites barely change.
pub(crate) struct ShellRenderer {
    pub(crate) compositor: Compositor,
    pub(crate) surface: Surface,
    pub(crate) backend: Renderer,
}

impl ShellRenderer {
    pub(crate) fn present(
        &mut self,
        viewport: &Viewport,
        background_color: iced::Color,
    ) -> Result<(), iced_tiny_skia::graphics::compositor::SurfaceError> {
        self.compositor.present(
            &mut self.backend,
            &mut self.surface,
            viewport,
            background_color,
            || {},
        )
    }

    pub(crate) fn resize(&mut self, width: u32, height: u32) {
        // Zero-size configures poison the next present (tiny-skia
        // pixel-map mismatch): skip until a real size arrives.
        if width == 0 || height == 0 {
            return;
        }
        self.compositor
            .configure_surface(&mut self.surface, width, height);
    }
}

/// Program UI instance plus its manual lifetime. Shared by the
/// debugger and PPU viewer. `Option` lets resize move the interface
/// through consuming `relayout` (panic leaves `None`, never garbage).
/// Pre-condition: `ui` borrows the boxed `instance` (stable address).
/// Post-condition: explicit `Drop` releases `ui` first, order-proof.
pub(crate) struct UiShell<P: Program<Theme = iced::Theme, Renderer = Renderer>> {
    ui: Option<UserInterface<'static, P::Message, iced::Theme, Renderer>>,
    /// Boxed: the interface borrows these bytes, so the address
    /// must survive moves of this struct (inline state would dangle).
    instance: Box<program::Instance<P>>,
}

impl<P: Program<Theme = iced::Theme, Renderer = Renderer>> Drop for UiShell<P> {
    fn drop(&mut self) {
        // Borrower before owner regardless of field order: `ui`
        // borrows `instance`, so release the interface first.
        self.ui = None;
    }
}

/// What a pumped event needs after `update`: nothing, a repaint,
/// or a view rebuild (`State::Outdated`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PumpOutcome {
    Idle,
    Repaint,
    Rebuild,
}

/// Map iced's update result onto [`PumpOutcome`]. Pure so the
/// message-less paths (wheel scroll especially) stay covered by
/// tests.
pub(crate) fn pump_outcome(state: &State) -> PumpOutcome {
    match state {
        State::Outdated => PumpOutcome::Rebuild,
        State::Updated { redraw_request, .. } if *redraw_request != RedrawRequest::Wait => {
            PumpOutcome::Repaint
        }
        State::Updated { .. } => PumpOutcome::Idle,
    }
}

impl<P: Program<Theme = iced::Theme, Renderer = Renderer>> UiShell<P> {
    /// Borrow the instance-bound interface with a caller-local lifetime.
    /// SAFETY: `'static` never escapes; `Box` fixes the address, drop order kills `ui` first.
    fn build_ui(
        instance: &program::Instance<P>,
        window_id: iced::window::Id,
        bounds: Size,
        cache: Cache,
        renderer: &mut Renderer,
    ) -> UserInterface<'static, P::Message, iced::Theme, Renderer> {
        unsafe {
            std::mem::transmute::<
                UserInterface<'_, P::Message, iced::Theme, Renderer>,
                UserInterface<'static, P::Message, iced::Theme, Renderer>,
            >(UserInterface::build(
                instance.view(window_id),
                bounds,
                cache,
                renderer,
            ))
        }
    }

    pub(crate) fn new(
        instance: program::Instance<P>,
        window_id: iced::window::Id,
        bounds: Size,
        renderer: &mut Renderer,
    ) -> Self {
        let instance = Box::new(instance);
        let ui = Self::build_ui(&instance, window_id, bounds, Cache::default(), renderer);
        Self {
            ui: Some(ui),
            instance,
        }
    }

    /// Borrow the live interface, or `None` after a panicked
    /// relayout dropped it. Callers log and bail; never unwrap.
    pub(crate) fn ui_mut(
        &mut self,
    ) -> Option<&mut UserInterface<'static, P::Message, iced::Theme, Renderer>> {
        self.ui.as_mut()
    }

    /// Push one program message without a rebuild (Sync-style refresh
    /// entry point shared by both viewers). The task is intentionally
    /// dropped: every aux program returns `Task::none` (sync-only).
    pub(crate) fn push_message(&mut self, message: P::Message) {
        let _task = self.instance.update(message);
    }

    /// What a pumped event needs after `update`: nothing, a repaint
    /// (wheel/drag/hover/caret move widget state but emit no
    /// messages), or a view rebuild (`State::Outdated`: the tree must
    /// be re-rendered from program state).
    pub(crate) fn pump(
        &mut self,
        mapped: iced::Event,
        cursor: mouse::Cursor,
        clipboard: &mut Clipboard,
        backend: &mut Renderer,
    ) -> (Vec<P::Message>, PumpOutcome) {
        let mut messages = Vec::new();
        let Some(ui) = self.ui_mut() else {
            log::warn!("aux window ui unavailable after panicked relayout");
            return (messages, PumpOutcome::Idle);
        };
        let (state, _) = ui.update(&[mapped], cursor, backend, clipboard, &mut messages);
        (messages, pump_outcome(&state))
    }

    /// Relayout for a new size: one layout pass reusing the retained
    /// tree and widget state (scroll, inputs, focus survive). No view
    /// re-invocation.
    pub(crate) fn relayout(&mut self, bounds: Size, renderer: &mut Renderer) {
        let Some(owned) = self.ui.take() else {
            return;
        };
        let rebuilt = owned.relayout(bounds, renderer);
        self.ui = Some(rebuilt);
    }

    /// Runs pending messages, rebuilding once. Reports whether a
    /// rebuild happened so the shell redraws only on real change.
    pub(crate) fn process_messages(
        &mut self,
        messages: Vec<P::Message>,
        window_id: iced::window::Id,
        bounds: Size,
        renderer: &mut Renderer,
        invalidated: &AtomicBool,
    ) -> bool {
        if messages.is_empty() && !invalidated.load(Ordering::Acquire) {
            return false;
        }
        // Single build: take the old UI out, harvest its cache, run
        // the messages, and rebuild once.
        let cache = self
            .ui
            .take()
            .map_or_else(Cache::default, UserInterface::into_cache);
        for msg in messages {
            let _task = self.instance.update(msg);
        }
        let rebuilt = Self::build_ui(&self.instance, window_id, bounds, cache, renderer);
        self.ui = Some(rebuilt);
        invalidated.store(false, Ordering::Release);
        true
    }
}

/// Window-system half shared by the debugger and PPU viewer
/// (twins by design): native window, viewport, input state, renderer,
/// and UI shell. Program-specific halves stay in the handles.
pub(crate) struct AuxShell<P: Program<Theme = iced::Theme, Renderer = Renderer>> {
    window: Arc<TaoWindow>,
    window_id: iced::window::Id,
    ui_state: UiShell<P>,
    renderer: ShellRenderer,
    viewport_physical: (u32, u32),
    /// Last surface-configured size. Resize storms only relayout;
    /// the realloc happens in `render`, once per presented frame.
    surface_size: (u32, u32),
    scale_factor: f32,
    modifiers: iced::keyboard::Modifiers,
    should_close: Arc<AtomicBool>,
    cursor: mouse::Cursor,
    clipboard: Clipboard,
}

impl<P: Program<Theme = iced::Theme, Renderer = Renderer>> AuxShell<P> {
    /// Native window identity for event routing.
    pub(crate) fn tao_window_id(&self) -> tao::window::WindowId {
        self.window.id()
    }

    /// Close was requested through the shared flag.
    pub(crate) fn close_requested(&self) -> bool {
        self.should_close.load(std::sync::atomic::Ordering::Acquire)
    }

    /// Native redraw request without reaching into the window.
    pub(crate) fn request_redraw(&self) {
        self.window.request_redraw();
    }

    /// Bring this window to the front.
    pub(crate) fn focus_window(&self) {
        self.window.set_focus();
    }

    /// Queue one program message for the next process pass.
    pub(crate) fn push_message(&mut self, message: P::Message) {
        self.ui_state.push_message(message);
    }

    /// Pump one converted event through the retained tree. Repaints
    /// redraw immediately; rebuilds report for the caller, which owns
    /// its invalidation flag.
    pub(crate) fn pump_frame(&mut self, mapped: iced::Event) -> (Vec<P::Message>, PumpOutcome) {
        let (messages, outcome) = self.ui_state.pump(
            mapped,
            self.cursor,
            &mut self.clipboard,
            &mut self.renderer.backend,
        );
        if matches!(outcome, PumpOutcome::Repaint) {
            self.window.request_redraw();
        }
        (messages, outcome)
    }

    /// Run pending messages against the retained tree, redrawing only
    /// on a real rebuild. Quiet batches (no messages, no flag) skip
    /// the frame: the trailing redraw in `pump_window_event` stays the
    /// safety net for paths that bypass `process`.
    /// Run pending messages; returns whether this pass requested a
    /// redraw (rebuild), so the event pump can skip its trailing
    /// request instead of queueing a duplicate present.
    pub(crate) fn process(&mut self, messages: Vec<P::Message>, invalidated: &AtomicBool) -> bool {
        let bounds = self.logical_bounds();
        let rebuilt = self.ui_state.process_messages(
            messages,
            self.window_id,
            bounds,
            &mut self.renderer.backend,
            invalidated,
        );
        if rebuilt {
            self.window.request_redraw();
        }
        rebuilt
    }
}

impl<P: Program<Theme = iced::Theme, Renderer = Renderer>> AuxShell<P> {
    /// Assemble from a scaffold plus a program instance. The close
    /// flag is a parameter (not minted here) so sharing programs
    /// observe the same flag.
    pub(crate) fn assemble(
        scaffold: WindowScaffold,
        instance: program::Instance<P>,
        should_close: Arc<AtomicBool>,
    ) -> Self {
        let WindowScaffold {
            window,
            window_id,
            scale_factor,
            viewport_physical,
            bounds,
            compositor,
            mut renderer,
            surface,
        } = scaffold;
        let ui_state = UiShell::new(instance, window_id, bounds, &mut renderer);
        window.request_redraw();
        Self {
            window,
            window_id,
            ui_state,
            renderer: ShellRenderer {
                compositor,
                surface,
                backend: renderer,
            },
            viewport_physical,
            // The scaffold builds the surface at this size.
            surface_size: viewport_physical,
            scale_factor,
            modifiers: iced::keyboard::Modifiers::default(),
            should_close,
            cursor: mouse::Cursor::default(),
            clipboard: Clipboard::unconnected(),
        }
    }

    /// Resize the raster and relayout the retained tree against the
    /// fresh bounds (no view rebuild). The surface realloc is deferred
    /// to `render` (one configure per presented frame); same-size
    /// events skip everything.
    pub(crate) fn resize(&mut self, width: u32, height: u32) {
        if self.viewport_physical == (width, height) {
            return;
        }
        self.viewport_physical = (width, height);
        log::debug!(
            "aux resize: physical={width}x{height} scale={} bounds={:?}",
            self.scale_factor,
            logical_bounds((width, height), self.scale_factor)
        );
        let bounds = logical_bounds((width, height), self.scale_factor);
        self.ui_state.relayout(bounds, &mut self.renderer.backend);
        self.window.request_redraw();
    }

    /// Scale changes move every layout bound: relayout like resize.
    pub(crate) fn set_scale_factor(&mut self, scale_factor: f32) {
        self.scale_factor = scale_factor;
        let bounds = logical_bounds(self.viewport_physical, self.scale_factor);
        self.ui_state.relayout(bounds, &mut self.renderer.backend);
        self.window.request_redraw();
    }

    pub(crate) fn set_modifiers(&mut self, modifiers: tao::keyboard::ModifiersState) {
        self.modifiers = crate::convert::tao_modifiers_to_iced(modifiers);
    }

    /// Shared event prelude for both twins (resize/scale/
    /// modifiers/close): byte-identical in the debugger and the PPU
    /// viewer, so it lives here instead of twice inline.
    pub(crate) fn handle_common_event(&mut self, event: &tao::event::WindowEvent) {
        match event {
            tao::event::WindowEvent::Resized(size) => self.resize(size.width, size.height),
            tao::event::WindowEvent::ScaleFactorChanged { scale_factor, .. } => {
                self.set_scale_factor(*scale_factor as f32);
            }
            tao::event::WindowEvent::ModifiersChanged(state) => self.set_modifiers(*state),
            tao::event::WindowEvent::CloseRequested => {
                self.should_close
                    .store(true, std::sync::atomic::Ordering::Release);
            }
            _ => {}
        }
    }

    /// Feed one native event, returning the converted iced event
    /// for the window's own dispatcher (debugger paging vs PPU
    /// hover). Returns the event instead of taking a closure so
    /// callers never double-borrow the shell.
    pub(crate) fn feed_tao_event(&mut self, event: tao::event::WindowEvent) -> Option<iced::Event> {
        crate::convert::convert_tao_window_event(
            event,
            &mut self.cursor,
            self.scale_factor,
            &mut self.modifiers,
            &self.should_close,
        )
    }

    /// Logical layout bounds for the current viewport and scale.
    pub(crate) fn logical_bounds(&self) -> Size {
        logical_bounds(self.viewport_physical, self.scale_factor)
    }
    /// Full render through the shared pump; only the log tag differs.
    pub(crate) fn render(&mut self, log_tag: &'static str) {
        // Self-healing scale: re-query the live factor and relayout
        // on drift (missed/late ScaleFactorChanged), so layout and
        // surface re-converge on the next frame.
        let live_scale = self.window.scale_factor() as f32;
        if live_scale != self.scale_factor {
            log::debug!(
                "{log_tag} render: scale drift {} -> {}; relayout",
                self.scale_factor,
                live_scale
            );
            self.scale_factor = live_scale;
            let bounds = logical_bounds(self.viewport_physical, self.scale_factor);
            self.ui_state.relayout(bounds, &mut self.renderer.backend);
        }
        // Deferred surface configure (see `resize`): resize storms
        // only relayout per event; the realloc lands here, at most
        // once per presented frame.
        if self.surface_size != self.viewport_physical {
            self.renderer
                .resize(self.viewport_physical.0, self.viewport_physical.1);
            self.surface_size = self.viewport_physical;
        }
        self.render_ui(log_tag);
    }

    /// Shared full render: Light theme, redraw pump, draw, present.
    /// Method on the shell so viewers never destructure it.
    fn render_ui(&mut self, log_tag: &'static str) {
        // Nothing to draw into yet (minimize/dock transient):
        // presenting a zero-size surface panics in pixel-map creation.
        if self.viewport_physical.0 == 0 || self.viewport_physical.1 == 0 {
            return;
        }
        let theme = iced::Theme::Light;
        let style = <iced::Theme as theme::Base>::base(&theme);
        let viewport = Viewport::with_physical_size(
            Size::new(self.viewport_physical.0, self.viewport_physical.1),
            self.scale_factor,
        );
        let redraw_event = iced::Event::Window(iced::window::Event::RedrawRequested(
            std::time::Instant::now(),
        ));
        let Some(ui) = self.ui_state.ui_mut() else {
            log::warn!("aux window ui unavailable after panicked relayout");
            return;
        };
        // Redraw-only pump: widget rebuild requests wait for the next
        // event-driven pump (`pump_outcome` mapping lives there).
        let _ = ui.update(
            &[redraw_event],
            self.cursor,
            &mut self.renderer.backend,
            &mut self.clipboard,
            &mut std::vec::Vec::new(),
        );
        ui.draw(
            &mut self.renderer.backend,
            &theme,
            &renderer::Style {
                text_color: style.text_color,
            },
            self.cursor,
        );
        if let Err(error) = self.renderer.present(&viewport, style.background_color) {
            log::warn!("{log_tag} render present failed: {error}");
        }
    }
}

/// Uniform aux-window event feed: shell prelude, program dispatch,
/// redraw request. Each handle keeps its own routing inside, so
/// dispatch arms stay orchestration-only (focus/drain/close).
pub(crate) trait AuxWindow {
    fn pump_window_event(&mut self, event: tao::event::WindowEvent);
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
        let mut backend = Renderer::new(crate::convert::default_font(), iced::Pixels(16.0));
        let shell = UiShell::new(
            instance,
            iced::window::Id::unique(),
            iced::Size::new(800.0, 600.0),
            &mut backend,
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
    fn pump_on_missing_ui_warns_and_idles() {
        let (mut shell, mut backend, mut clipboard) = stub_shell();
        let _ = shell.ui.take();
        assert!(shell.ui_mut().is_none());
        let (messages, outcome) = shell.pump(
            iced::Event::Window(iced::window::Event::RedrawRequested(
                std::time::Instant::now(),
            )),
            iced::mouse::Cursor::default(),
            &mut clipboard,
            &mut backend,
        );
        assert!(messages.is_empty());
        assert_eq!(outcome, PumpOutcome::Idle);
        // Push and relayout stay total without an interface.
        shell.push_message(StubMessage::Ping);
        shell.relayout(iced::Size::new(640.0, 480.0), &mut backend);
        assert!(shell.ui_mut().is_none());
    }

    #[test]
    fn ui_drops_before_instance_on_both_shell_shapes() {
        // Exercises the explicit `Drop`: live and relayout-orphaned
        // (`None`) shells both drop cleanly (Miri/ASan catch order bugs).
        let (shell, _, _) = stub_shell();
        drop(shell);
        let (mut shell, _, _) = stub_shell();
        let _ = shell.ui.take();
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
        let built = VIEW_CALLS.load(Ordering::SeqCst);
        assert!(built > 0);
        // Empty batch without invalidation: no rebuild.
        let still = AtomicBool::new(false);
        shell.process_messages(
            Vec::new(),
            iced::window::Id::unique(),
            iced::Size::new(800.0, 600.0),
            &mut backend,
            &still,
        );
        assert_eq!(VIEW_CALLS.load(Ordering::SeqCst), built);
        // Either a message or the flag rebuilds exactly once.
        shell.process_messages(
            vec![StubMessage::Ping],
            iced::window::Id::unique(),
            iced::Size::new(800.0, 600.0),
            &mut backend,
            &still,
        );
        assert_eq!(VIEW_CALLS.load(Ordering::SeqCst), built + 1);
        let flagged = AtomicBool::new(true);
        shell.process_messages(
            Vec::new(),
            iced::window::Id::unique(),
            iced::Size::new(800.0, 600.0),
            &mut backend,
            &flagged,
        );
        assert_eq!(VIEW_CALLS.load(Ordering::SeqCst), built + 2);
        assert!(!flagged.load(Ordering::Acquire));
    }
}
