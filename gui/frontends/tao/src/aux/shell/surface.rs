//! Shared iced-on-tao auxiliary-window shell: debugger and PPU viewer
//! run the same tiny-skia lifecycle over different programs.

use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use iced::{Rectangle, Size, mouse};
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

#[cfg(target_os = "linux")]
use gtk::prelude::{BoxExt as _, ImageExt as _, WidgetExt as _};

#[cfg(target_os = "linux")]
use tao::platform::unix::WindowExtUnix;

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
    surface: Option<Surface>,
    backend: Renderer,
    /// SPIKE NER-335: GTK3 image blit target. `Some` once seated in
    /// the tao vbox; then presents go through GdkPixbuf, never softbuffer.
    #[cfg(target_os = "linux")]
    image: Option<gtk::Image>,
    /// Last surface-configured size. Resize storms only relayout;
    /// the realloc happens in `render`, once per presented frame.
    surface_size: (u32, u32),
    modifiers: iced::keyboard::Modifiers,
    should_close: Arc<AtomicBool>,
    cursor: mouse::Cursor,
    clipboard: Clipboard,
    retry: nerust_frontend_kernel::retry::RenderRetry,
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
        // SPIKE NER-335: seat a GtkImage in the tao vbox so the iced UI
        // composites as a GTK widget under the native frame. Softbuffer
        // stays as fallback when the vbox is unavailable.
        #[cfg(target_os = "linux")]
        let image = window.default_vbox().map(|vbox| {
            let image = gtk::Image::new();
            image.set_hexpand(true);
            image.set_vexpand(true);
            vbox.pack_end(&image, true, true, 0);
            image.show();
            log::debug!("{log_tag} image-blit seated in vbox");
            image
        });
        let viewport_physical = (window_size.width, window_size.height);
        let logical_size = window_size.to_logical::<f64>(scale_factor as f64);
        let bounds = Size::new(logical_size.width as f32, logical_size.height as f32);
        #[cfg_attr(target_os = "linux", allow(unused_mut))]
        let mut compositor = compositor::new(
            iced_tiny_skia::Settings {
                default_font: default_font(),
                default_text_size: iced::Pixels(16.0),
            },
            Arc::clone(&window),
        );
        let renderer = compositor.create_renderer();
        // SPIKE NER-335: the pixbuf path needs no wl_surface; skip the
        // softbuffer surface so iced never attaches to the GDK surface.
        #[cfg(target_os = "linux")]
        let surface: Option<Surface> = None;
        #[cfg(not(target_os = "linux"))]
        let surface = Some(compositor.create_surface(
            Arc::clone(&window),
            window_size.width,
            window_size.height,
        ));
        Ok(Self {
            window,
            window_id,
            scale_factor,
            viewport_physical,
            bounds,
            compositor,
            backend: renderer,
            surface,
            #[cfg(target_os = "linux")]
            image,
            surface_size: viewport_physical,
            modifiers: iced::keyboard::Modifiers::default(),
            should_close,
            cursor: mouse::Cursor::default(),
            clipboard: Clipboard::unconnected(),
            retry: Default::default(),
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
            self.logical_bounds()
        );
        let bounds = self.logical_bounds();
        relayout(bounds);
        self.window.request_redraw();
    }

    /// Scale changes move every layout bound: record like resize.
    pub(super) fn set_scale_factor(&mut self, scale_factor: f32, relayout: &mut impl FnMut(Size)) {
        self.scale_factor = scale_factor;
        let bounds = self.logical_bounds();
        relayout(bounds);
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
            &mut crate::convert::FeedContext {
                cursor: &mut self.cursor,
                modifiers: &mut self.modifiers,
                scale_factor: self.scale_factor,
                should_close: &self.should_close,
            },
        )
    }
    /// Logical layout bounds for the current viewport and scale.
    /// SPIKE NER-335: with the image blit seated, layout follows the
    /// vbox client rect (tao's inner size is CSD-inclusive on Wayland).
    pub(super) fn logical_bounds(&self) -> Size {
        #[cfg(target_os = "linux")]
        if self.image.is_some() {
            return self.present_viewport().logical_size();
        }
        logical_bounds(self.viewport_physical, self.scale_factor)
    }

    /// Viewport the next present rasterizes into. With the image blit
    /// seated this is the vbox client rect times the scale factor, so
    /// the pixmap matches the GtkImage allocation 1:1 at scale 1 and
    /// downscales from full physical resolution above it.
    pub(super) fn present_viewport(&self) -> Viewport {
        #[cfg(target_os = "linux")]
        if self.image.is_some() {
            let (width, height) = self.client_physical();
            if width > 0 && height > 0 {
                return Viewport::with_physical_size(Size::new(width, height), self.scale_factor);
            }
        }
        Viewport::with_physical_size(
            Size::new(self.viewport_physical.0, self.viewport_physical.1),
            self.scale_factor,
        )
    }

    /// SPIKE NER-335: raster size from the vbox client rect. Falls back
    /// to the tao inner size pre-map or when the vbox is unavailable
    /// (X11/SSD: identical; Wayland CSD: excludes decorations).
    #[cfg(target_os = "linux")]
    fn client_physical(&self) -> (u32, u32) {
        match vbox_logical(&self.window) {
            Some((width, height)) => (
                (f64::from(width) * f64::from(self.scale_factor)).round() as u32,
                (f64::from(height) * f64::from(self.scale_factor)).round() as u32,
            ),
            None => self.viewport_physical,
        }
    }

    /// Surface realloc, at most once per frame.
    /// SPIKE NER-335: the pixbuf path owns no surface; skip the realloc
    /// and let the image blit reallocate per frame instead.
    pub(super) fn configure_surface(&mut self) {
        #[cfg(target_os = "linux")]
        if self.image.is_some() {
            return;
        }
        let (width, height) = self.viewport_physical;
        if width == 0 || height == 0 {
            return;
        }
        if self.surface.is_none() {
            self.surface = Some(self.compositor.create_surface(
                Arc::clone(&self.window),
                width,
                height,
            ));
            self.surface_size = self.viewport_physical;
        } else if self.surface_size != self.viewport_physical {
            if let Some(surface) = self.surface.as_mut() {
                self.compositor.configure_surface(surface, width, height);
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
            let bounds = self.logical_bounds();
            relayout(bounds);
        }
    }

    pub(super) fn present(
        &mut self,
        viewport: &Viewport,
        background_color: iced::Color,
        log_tag: &'static str,
    ) -> Option<String> {
        // SPIKE NER-335: blit through the GTK image when seated;
        // softbuffer stays as fallback when the vbox was unavailable.
        #[cfg(target_os = "linux")]
        if let Some(image) = self.image.clone() {
            return self.present_pixbuf(&image, viewport, background_color, log_tag);
        }
        if let Err(error) = self.compositor.present(
            &mut self.backend,
            self.surface.as_mut()?,
            viewport,
            background_color,
            || {},
        ) {
            self.surface = None;
            if self.retry.failed(std::time::Instant::now()) {
                let detail =
                    format!("{log_tag} render present failed; recreating with backoff: {error}");
                log::error!("{detail}");
                return Some(detail);
            }
        } else {
            self.retry.succeeded();
        }
        None
    }

    /// SPIKE NER-335: memory-render the iced UI and hand the bytes to
    /// GTK as a GdkPixbuf. Mirrors `window::compositor::screenshot`:
    /// full-damage draw into a u32 pixel map, then channel extraction
    /// (opaque-correct; semi-transparent blends stay premultiplied).
    #[cfg(target_os = "linux")]
    fn present_pixbuf(
        &mut self,
        image: &gtk::Image,
        viewport: &Viewport,
        background_color: iced::Color,
        log_tag: &'static str,
    ) -> Option<String> {
        use gtk::gdk_pixbuf::{Colorspace, Pixbuf};
        let physical = viewport.physical_size();
        let (width, height) = (physical.width, physical.height);
        if width == 0 || height == 0 {
            return None;
        }
        // Damage is logical: draw() scales it by the viewport factor
        // internally (upstream present passes logical_size too).
        let logical = viewport.logical_size();
        let mut raw = vec![0u8; width as usize * height as usize * 4];
        let drawn =
            tiny_skia::PixmapMut::from_bytes(&mut raw, width, height).and_then(|mut pixels| {
                tiny_skia::Mask::new(width, height).map(|mut mask| {
                    let damage = [Rectangle::with_size(logical)];
                    self.backend
                        .draw(&mut pixels, &mut mask, viewport, &damage, background_color);
                })
            });
        if drawn.is_none() {
            log::error!("{log_tag} pixbuf present: pixel map alloc failed");
            return Some(format!("{log_tag} pixbuf pixel map alloc failed"));
        }
        // ARGB u32 words in little-endian memory order B, G, R, A.
        let mut rgba = Vec::with_capacity(raw.len());
        let (words, _) = raw.as_chunks::<4>();
        for word in words {
            let pixel = u32::from_le_bytes(*word);
            rgba.push(((pixel >> 16) & 0xFF) as u8);
            rgba.push(((pixel >> 8) & 0xFF) as u8);
            rgba.push((pixel & 0xFF) as u8);
            rgba.push(((pixel >> 24) & 0xFF) as u8);
        }
        let pixbuf = Pixbuf::from_mut_slice(
            rgba,
            Colorspace::Rgb,
            true,
            8,
            width as i32,
            height as i32,
            (width * 4) as i32,
        );
        // GTK3 images show natural pixels: downscale physical pixbufs to
        // logical size so HiDPI allocations match the vbox. Rounded:
        // truncation would shrink fractional-scale clients by a pixel.
        let (target_w, target_h) = (logical.width.round() as u32, logical.height.round() as u32);
        let pixbuf = if (target_w, target_h) != (width, height) {
            pixbuf
                .scale_simple(
                    target_w as i32,
                    target_h as i32,
                    gtk::gdk_pixbuf::InterpType::Bilinear,
                )
                .unwrap_or(pixbuf)
        } else {
            pixbuf
        };
        image.set_from_pixbuf(Some(&pixbuf));
        self.retry.succeeded();
        None
    }

    pub(super) fn retry_deadline(&self) -> Option<std::time::Instant> {
        self.retry.deadline()
    }
    pub(super) fn render_ready(&self) -> bool {
        self.retry.ready(std::time::Instant::now())
    }
    pub(super) fn retry_due(&mut self, now: std::time::Instant) -> bool {
        self.retry.take_due(now)
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
}

/// SPIKE NER-335: true client size from the GTK content box, logical px.
/// tao's inner size is CSD-inclusive on Wayland CSD fallback; the vbox
/// is the real client rect. `None` pre-map or when unavailable.
#[cfg(target_os = "linux")]
fn vbox_logical(window: &TaoWindow) -> Option<(i32, i32)> {
    let vbox = window.default_vbox()?;
    let alloc = vbox.allocation();
    (alloc.width() >= 8 && alloc.height() >= 8).then_some((alloc.width(), alloc.height()))
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
