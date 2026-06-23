use super::State;
use super::renderer::GtkGlRenderer;
use gtk::glib;
use gtk::prelude::*;
use nerust_screen_video::SurfaceSize;
use raw_window_handle::{
    RawDisplayHandle, RawWindowHandle, WaylandWindowHandle, XlibDisplayHandle,
};
use std::cell::RefCell;
use std::rc::Rc;

pub(crate) struct SurfaceCore {
    window: gtk::ApplicationWindow,
    renderer: Rc<RefCell<GtkGlRenderer>>,
    state: Rc<RefCell<State>>,
}

pub(crate) type Surface = Rc<RefCell<SurfaceCore>>;

pub(crate) trait SurfaceExtend {
    fn bind(window: &gtk::ApplicationWindow, state: Rc<RefCell<State>>) -> Surface;
    fn tick(&self) -> bool;
}

/// Extract a raw window handle from a GDK surface.
fn gdk_surface_to_raw(surface: &gdk::Surface) -> Option<RawWindowHandle> {
    // GTK4 on Linux uses either X11 or Wayland. We try to determine
    // the backend by checking the surface type at runtime.
    let ptr = surface.as_ptr() as *mut std::ffi::c_void;

    // Try Wayland first — less overhead if it's available.
    // The wl_surface pointer is typically at the first field of GdkWaylandSurface.
    // This is a best-effort detection; actual backend depends on GTK configuration.
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        // For Wayland, we use the surface pointer directly as wl_surface*.
        // In GDK, GdkWaylandSurface's first field is GdkSurface, second is wl_surface.
        // This is fragile; ideally we'd use gdk4-wayland crate.
        let wayland = WaylandWindowHandle::new(std::ptr::NonNull::new(ptr).unwrap());
        return Some(RawWindowHandle::Wayland(wayland));
    }

    #[cfg(not(all(unix, not(target_os = "macos"))))]
    {
        let _ = ptr;
        None
    }
}

/// Extract a raw display handle from a GDK display.
fn gdk_display_to_raw(display: &gdk::Display) -> Option<RawDisplayHandle> {
    let ptr = display.as_ptr() as *mut std::ffi::c_void;

    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let xlib = XlibDisplayHandle::new(std::ptr::NonNull::new(ptr), 0);
        return Some(RawDisplayHandle::Xlib(xlib));
    }

    #[cfg(not(all(unix, not(target_os = "macos"))))]
    {
        let _ = ptr;
        None
    }
}

impl SurfaceExtend for Surface {
    fn bind(window: &gtk::ApplicationWindow, state: Rc<RefCell<State>>) -> Surface {
        let renderer = Rc::new(RefCell::new(GtkGlRenderer::new()));
        let result = Rc::new(RefCell::new(SurfaceCore {
            window: window.clone(),
            renderer: renderer.clone(),
            state,
        }));
        {
            let result = result.clone();
            let _ = window.add_tick_callback(move |_window, _frame_clock| {
                glib::ControlFlow::from(result.tick())
            });
        }
        result
    }

    fn tick(&self) -> bool {
        let mut needs_reinit = false;
        if let Ok(mut state) = self.borrow().state.try_borrow_mut() {
            needs_reinit = state.take_renderer_reload_pending();
            state.swap_frame_buffer();
        }
        if needs_reinit && let Ok(state) = self.borrow().state.try_borrow() {
            let size = SurfaceSize::new(
                self.borrow().window.width() as u32,
                self.borrow().window.height() as u32,
            );
            let profile = state.render_profile().clone();
            if let Some(surface) = self.borrow().window.surface()
                && let Some(display) = gdk::Display::default()
                && let Some(wh) = gdk_surface_to_raw(&surface)
                && let Some(dh) = gdk_display_to_raw(&display)
            {
                self.borrow()
                    .renderer
                    .borrow_mut()
                    .realize(wh, dh, size, &profile);
            }
        }
        if let Ok(state) = self.borrow().state.try_borrow() {
            self.borrow()
                .renderer
                .borrow_mut()
                .render(state.frame_buffer());
        }
        true
    }
}
