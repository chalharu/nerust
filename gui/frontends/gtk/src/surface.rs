use super::State;
use super::renderer::GtkRenderer;
use gtk::glib;
use gtk::prelude::*;
use nerust_screen_video::SurfaceSize;
use raw_window_handle::{
    RawDisplayHandle, RawWindowHandle, WaylandWindowHandle, XlibDisplayHandle,
};
use std::cell::RefCell;
use std::ffi::c_void;
use std::ptr::NonNull;
use std::rc::Rc;

pub(crate) struct SurfaceCore {
    window: gtk::ApplicationWindow,
    renderer: Rc<RefCell<GtkRenderer>>,
    state: Rc<RefCell<State>>,
}

pub(crate) type Surface = Rc<RefCell<SurfaceCore>>;

pub(crate) trait SurfaceExtend {
    fn bind(window: &gtk::ApplicationWindow, state: Rc<RefCell<State>>) -> Surface;
    fn tick(&self) -> bool;
}

/// On non-Linux platforms GTK is not available, so raw handle extraction
/// always fails. The cfg is consolidated here rather than duplicated in
/// each extraction function.
#[cfg(all(unix, not(target_os = "macos")))]
fn gdk_ptr(ptr: *mut c_void) -> Option<NonNull<c_void>> {
    NonNull::new(ptr)
}

#[cfg(not(all(unix, not(target_os = "macos"))))]
fn gdk_ptr(_ptr: *mut c_void) -> Option<NonNull<c_void>> {
    None
}

fn gdk_surface_to_raw(surface: &gdk::Surface) -> Option<RawWindowHandle> {
    let ptr = gdk_ptr(surface.as_ptr() as *mut c_void)?;
    Some(RawWindowHandle::Wayland(WaylandWindowHandle::new(ptr)))
}

fn gdk_display_to_raw(display: &gdk::Display) -> Option<RawDisplayHandle> {
    let ptr = gdk_ptr(display.as_ptr() as *mut c_void)?;
    Some(RawDisplayHandle::Xlib(XlibDisplayHandle::new(Some(ptr), 0)))
}

impl SurfaceExtend for Surface {
    fn bind(window: &gtk::ApplicationWindow, state: Rc<RefCell<State>>) -> Surface {
        let renderer = Rc::new(RefCell::new(GtkRenderer::new()));
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
