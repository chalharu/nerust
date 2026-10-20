//! Program presenter: one surface plus one program shell per aux window.
//! Program-specific halves stay in the handles.

use std::sync::atomic::{AtomicBool, Ordering};

use iced::{Size, advanced::renderer, theme};
use iced_tiny_skia::Renderer;
use iced_winit::{
    graphics::Viewport,
    program::{self, Program},
    runtime::user_interface::UserInterface,
};

use super::AuxShellOps;
use super::pump::{PumpOutcome, UiShell};
use super::surface::AuxSurface;

/// One window-system surface plus one program shell.
/// Program-specific halves stay in the handles.
pub(crate) struct AuxShell<P: Program<Theme = iced::Theme, Renderer = Renderer>> {
    surface: AuxSurface,
    ui_state: UiShell<P>,
}

impl<P: Program<Theme = iced::Theme, Renderer = Renderer>> AuxShellOps for AuxShell<P> {
    fn native_id(&self) -> crate::aux::domain::NativeWindowId {
        self.surface.native_id()
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

    fn handle_common(&mut self, event: &crate::aux::domain::AuxNativeEvent) {
        let ui_state = &mut self.ui_state;
        self.surface
            .handle_common_event(event, &mut |bounds| ui_state.relayout(bounds));
    }

    fn feed_event(&mut self, event: crate::aux::domain::AuxNativeEvent) -> Option<iced::Event> {
        self.surface.feed_domain_event(event)
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
        let (backend, cursor, clipboard) = self.surface.draw_parts();
        let outcome = self.ui_state.pump(mapped, cursor, clipboard, backend, out);
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
        let mut out = self.ui_state.take_scratch();
        let outcome = self.pump_frame(mapped, &mut out);
        out.extend(extra);
        let done = self.finish(outcome, &mut out, invalidated);
        self.ui_state.restore_scratch(out);
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
                .process_messages(messages, self.surface.backend_mut(), invalidated);
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
        let (window_id, bounds) = (surface.window_id(), surface.bounds());
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
        let (width, height) = self.surface.viewport();
        if width == 0 || height == 0 {
            return;
        }
        let theme = iced::Theme::Light;
        let style = <iced::Theme as theme::Base>::base(&theme);
        let viewport = Viewport::with_physical_size(Size::new(width, height), self.surface.scale());
        let redraw_event = iced::Event::Window(iced::window::Event::RedrawRequested(
            std::time::Instant::now(),
        ));
        {
            let mut sink = self.ui_state.take_scratch();
            sink.clear();
            let tree = &mut self.ui_state.tree;
            let cache = std::mem::take(&mut tree.cache);
            let (backend, cursor, clipboard) = self.surface.draw_parts();
            let mut ui = UserInterface::build(
                tree.instance.view(tree.window_id),
                tree.bounds,
                cache,
                backend,
            );
            // Redraw-only pump: widget rebuild requests wait for the next
            // event-driven pump (`pump_outcome` mapping lives there).
            let _ = ui.update(&[redraw_event], cursor, backend, clipboard, &mut sink);
            ui.draw(
                backend,
                &theme,
                &renderer::Style {
                    text_color: style.text_color,
                },
                cursor,
            );
            tree.cache = ui.into_cache();
            self.ui_state.restore_scratch(sink);
        }
        self.surface
            .present(&viewport, style.background_color, log_tag);
    }
}
