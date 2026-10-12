//! Linux render target below the muda menubar.
//!
//! Background: on tao's gtk backend the muda menu is packed into the
//! toplevel's `default_vbox`, but the game renderer used to attach to
//! the toplevel window itself, so every present painted over the menu
//! (Windows puts the menu in the non-client area and macOS uses the
//! global menubar, so neither ever hit this). The fix renders into a
//! dedicated `EventBox` child: the menu keeps its own native pixels,
//! the game keeps everything below it.
//!
//! The `EventBox` owns a real X window on X11, so the existing
//! backends (softbuffer/wgpu/opengl) attach to it like any toplevel.
//! On Wayland there is no XID and before the first layout there is no
//! `GdkWindow` yet: both cases report `None` and callers fall back to
//! the toplevel (menu occluded, as before) instead of failing. A
//! later frame upgrades once the area materializes.

use std::sync::Arc;

use gtk::prelude::*;
use nerust_render_traits::SurfaceSize;
use raw_window_handle::{RawWindowHandle, XlibWindowHandle};
use tao::window::Window as TaoWindow;

/// Minimum believable content size: below this the area is not laid
/// out yet (pre-map allocation) and the toplevel fallback applies.
const MIN_CONTENT_EDGE: i32 = 8;

pub(crate) struct ContentArea {
    area: gtk::EventBox,
}

impl ContentArea {
    pub(crate) fn new(
        vbox: &gtk::Box,
        gtk_window: &gtk::ApplicationWindow,
        window: &Arc<TaoWindow>,
    ) -> Self {
        let area = gtk::EventBox::new();
        area.set_hexpand(true);
        area.set_vexpand(true);
        // GTK must not paint here: raw presents own every pixel, and
        // a background fill would flash over the game on exposes.
        area.connect_draw(|_, _| gtk::glib::Propagation::Stop);
        {
            let window = Arc::clone(window);
            area.connect_size_allocate(move |_, _| window.request_redraw());
        }
        vbox.pack_end(&area, true, true, 0);
        area.show();
        // Materialize the native window now so the first attach can
        // already target it; layout still arrives with the first map
        // and the per-frame size check converges then.
        area.realize();
        paint_window_background(gtk_window, &area);
        Self { area }
    }

    /// Native target (child XID plus its physical size), or `None`
    /// when unavailable. Pure query: safe to call every frame.
    pub(crate) fn target(&self) -> Option<(RawWindowHandle, SurfaceSize, u64)> {
        let gwin = self.area.window()?;
        let width = gwin.width();
        let height = gwin.height();
        if width < MIN_CONTENT_EDGE || height < MIN_CONTENT_EDGE {
            return None;
        }
        let xid = x11_xid(&gwin)?;
        let handle = RawWindowHandle::Xlib(XlibWindowHandle::new(xid));
        let size = SurfaceSize::new(width as u32, height as u32);
        Some((handle, size, xid))
    }
}

/// XID of a `GdkWindow`, or `None` outside X11. The downcast (not a
/// backend sniff) decides: Wayland windows simply fail it.
fn x11_xid(gwin: &gtk::gdk::Window) -> Option<u64> {
    use gtk::glib::object::Cast;
    let xwin: gdkx11::X11Window = gwin.clone().downcast().ok()?;
    Some(xwin.xid())
}

/// Fill the toplevel background with the theme rendering. tao marks
/// the window app-paintable (nothing paints it by default) and the
/// game now only covers the content child, so without this the
/// menubar strip shows uninitialized pixels. `Proceed` keeps children
/// (menu, content) painting; the content child's own native window
/// composites above regardless.
fn paint_window_background(gtk_window: &gtk::ApplicationWindow, _area: &gtk::EventBox) {
    gtk_window.connect_draw(|window, cr| {
        let context = window.style_context();
        let alloc = window.allocation();
        gtk::render_background(
            &context,
            cr,
            0.0,
            0.0,
            f64::from(alloc.width()),
            f64::from(alloc.height()),
        );
        gtk::glib::Propagation::Proceed
    });
    // Startup draws run pre-map (both windows are already mapped by
    // the time we connect) and an unobscured window gets no
    // follow-up expose, so re-queue once from the main loop: by then
    // mapping has completed and the paint sticks. Later exposes and
    // resizes maintain it from there.
    let pending = gtk_window.clone();
    gtk::glib::timeout_add_local(std::time::Duration::from_millis(300), move || {
        pending.queue_draw();
        gtk::glib::ControlFlow::Break
    });
}
