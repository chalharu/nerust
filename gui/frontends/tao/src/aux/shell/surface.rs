//! Shared iced-on-tao auxiliary-window shell: debugger and PPU viewer
//! run the same tiny-skia lifecycle over different programs.

use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use iced::{Size, mouse};
use iced_tiny_skia::{
    Renderer,
    graphics::compositor::Compositor as _,
    window::{Compositor, Surface, compositor},
};
use iced_winit::{Clipboard, graphics::Viewport};
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

/// Window creation plus the live window system in one surface.
/// Programs and UI instances stay per-window in [`UiShell`].
pub(crate) struct AuxSurface {
    window: Arc<TaoWindow>,
    window_id: iced::window::Id,
    scale_factor: f32,
    viewport_physical: (u32, u32),
    bounds: Size,
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
    pub(super) fn native_id(&self) -> crate::aux::domain::NativeWindowId {
        crate::aux::native::native_id_of(self.window.id())
    }

    /// Close was requested through the shared flag.
    pub(super) fn close_requested(&self) -> bool {
        self.should_close.load(Ordering::Acquire)
    }

    /// Native redraw request without reaching into the window.
    pub(super) fn request_redraw(&self) {
        self.window.request_redraw();
    }

    /// Bring this window to the front.
    pub(super) fn focus_window(&self) {
        self.window.set_focus();
    }

    /// Resize the raster and record fresh bounds (no view rebuild).
    /// The surface realloc is deferred to `render`; same-size events skip everything.
    pub(super) fn resize(&mut self, width: u32, height: u32, relayout: &mut impl FnMut(Size)) {
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
    pub(super) fn set_scale_factor(&mut self, scale_factor: f32, relayout: &mut impl FnMut(Size)) {
        self.scale_factor = scale_factor;
        relayout(logical_bounds(self.viewport_physical, self.scale_factor));
        self.window.request_redraw();
    }

    pub(super) fn set_modifiers(&mut self, modifiers: iced::keyboard::Modifiers) {
        self.modifiers = modifiers;
    }

    /// Shared event prelude (resize/scale/modifiers/close).
    pub(super) fn handle_common_event(
        &mut self,
        event: &crate::aux::domain::AuxNativeEvent,
        relayout: &mut impl FnMut(Size),
    ) {
        use crate::aux::domain::AuxNativeEvent as E;
        match event {
            E::Resized(width, height) => self.resize(*width, *height, relayout),
            E::ScaleChanged(scale_factor) => {
                self.set_scale_factor(*scale_factor, relayout);
            }
            E::ModifiersChanged(modifiers) => {
                self.set_modifiers(crate::convert::aux_modifiers_to_iced(*modifiers))
            }
            E::CloseRequested => {
                self.should_close.store(true, Ordering::Release);
            }
            _ => {}
        }
    }

    /// Feed one domain event, returning the converted iced event.
    pub(super) fn feed_domain_event(
        &mut self,
        event: crate::aux::domain::AuxNativeEvent,
    ) -> Option<iced::Event> {
        crate::convert::feed_domain_event(
            event,
            &mut self.cursor,
            &mut self.modifiers,
            self.scale_factor,
            &self.should_close,
        )
    }
    /// Logical layout bounds for the current viewport and scale.
    pub(super) fn logical_bounds(&self) -> Size {
        logical_bounds(self.viewport_physical, self.scale_factor)
    }

    /// Surface realloc, at most once per frame.
    pub(super) fn configure_surface(&mut self) {
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
    pub(super) fn heal_scale(&mut self, log_tag: &'static str, relayout: &mut impl FnMut(Size)) {
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

    pub(super) fn present(
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

    /// Window-system id for program instance binding.
    pub(super) fn window_id(&self) -> iced::window::Id {
        self.window_id
    }

    /// Current layout bounds for program instance binding.
    pub(crate) fn bounds(&self) -> Size {
        self.bounds
    }

    /// Physical viewport for frame setup (zero while minimized).
    pub(crate) fn viewport(&self) -> (u32, u32) {
        self.viewport_physical
    }

    /// Live scale factor for frame setup.
    pub(crate) fn scale(&self) -> f32 {
        self.scale_factor
    }

    /// Renderer plus input state for one draw pass.
    pub(super) fn draw_parts(&mut self) -> (&mut Renderer, mouse::Cursor, &mut Clipboard) {
        (&mut self.backend, self.cursor, &mut self.clipboard)
    }

    /// Renderer for message-run passes (no cursor/clipboard use).
    pub(super) fn backend_mut(&mut self) -> &mut Renderer {
        &mut self.backend
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn logical_bounds_divide_by_scale() {
        // Resize/scale rebuilds share this math: 2x shows half the
        // logical pixels of the physical viewport.
        let bounds = logical_bounds((1100, 800), 2.0);
        assert_eq!((bounds.width, bounds.height), (550.0, 400.0));
        let unit = logical_bounds((960, 720), 1.0);
        assert_eq!((unit.width, unit.height), (960.0, 720.0));
        // Zero viewports stay zero until the next pump rebuilds.
        assert_eq!(logical_bounds((0, 0), 2.0), iced::Size::new(0.0, 0.0));
        assert_eq!(logical_bounds((1, 1), 2.0), iced::Size::new(0.5, 0.5));
    }

    #[test]
    fn shared_chrome_helpers_are_stable() {
        assert!(matches!(aux_theme(), Some(iced::Theme::Light)));
        let settings = aux_settings();
        assert_eq!(settings.default_text_size, iced::Pixels(16.0));
    }

    #[test]
    fn scaffold_error_names_window_and_detail() {
        let error = ScaffoldError::BuildWindow {
            tag: "history window",
            detail: "no display".to_string(),
        };
        assert_eq!(
            error.to_string(),
            "native window build failed for history window: no display"
        );
        assert_eq!(
            format!("{error:?}"),
            "BuildWindow { tag: \"history window\", detail: \"no display\" }"
        );
    }
}
