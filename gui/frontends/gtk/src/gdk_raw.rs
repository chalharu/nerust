use std::ptr::NonNull;

use gdk::prelude::DisplayExtManual as _;
use gio::glib::object::ObjectType as _;
use gtk::gdk;
use raw_window_handle::{RawDisplayHandle, RawWindowHandle};

/// Extract handles and call `f` within a scope where the native objects are
/// guaranteed alive.
///
/// On macOS the NSWindow/NSView are retained for the duration of the closure,
/// then released.  Other platforms pass the raw handles through directly since
/// they are borrowed from GDK objects whose lifetime is bounded by the caller.
#[cfg(target_os = "macos")]
pub(crate) fn with_raw_handles<R>(
    surface: &gdk::Surface,
    display: &gdk::Display,
    f: impl FnOnce(RawWindowHandle, RawDisplayHandle) -> R,
) -> Option<R> {
    use std::ffi::c_void;

    use gio::prelude::Cast;

    let ns_window = surface.downcast_ref::<gdk_macos::MacosSurface>()?.native();
    // SAFETY: Retain the NSWindow so it (and its NSView) stay alive for `f`.
    let ns_window =
        unsafe { objc2::rc::Retained::<objc2_app_kit::NSWindow>::retain(ns_window.cast()) }?;
    let ns_view = ns_window.contentView()?;
    let wh = RawWindowHandle::AppKit(raw_window_handle::AppKitWindowHandle::new(
        NonNull::new(objc2::rc::Retained::as_ptr(&ns_view) as *mut c_void).unwrap(),
    ));
    let dh = display_to_raw(display)?;
    Some(f(wh, dh))
    // ns_window (and ns_view) drop here — the pointer is no longer used.
}

/// Extract handles and call `f` — GDK owns the objects, no retain needed.
#[cfg(not(target_os = "macos"))]
pub(crate) fn with_raw_handles<R>(
    surface: &gdk::Surface,
    display: &gdk::Display,
    f: impl FnOnce(RawWindowHandle, RawDisplayHandle) -> R,
) -> Option<R> {
    let wh = surface_to_raw(surface)?;
    let dh = display_to_raw(display)?;
    Some(f(wh, dh))
}

// ---------------------------------------------------------------------------
// Platform-specific surface_to_raw — kept private, called by surface_with.
// ---------------------------------------------------------------------------

#[cfg(all(not(unix), not(target_os = "windows")))]
fn surface_to_raw(_surface: &gdk::Surface) -> Option<RawWindowHandle> {
    None
}

#[cfg(all(unix, not(target_os = "macos")))]
fn surface_to_raw(surface: &gdk::Surface) -> Option<RawWindowHandle> {
    use gdk::prelude::SurfaceExt as _;

    let backend = surface.display().backend();
    let ptr = surface.as_ptr();

    if backend.is_x11() {
        let xid = unsafe { gdk4_x11_sys::gdk_x11_surface_get_xid(ptr.cast()) };
        return Some(RawWindowHandle::Xlib(
            raw_window_handle::XlibWindowHandle::new(xid),
        ));
    }

    if backend.is_wayland() {
        let wl_surface =
            unsafe { gdk4_wayland_sys::gdk_wayland_surface_get_wl_surface(ptr.cast()) };
        return Some(RawWindowHandle::Wayland(
            raw_window_handle::WaylandWindowHandle::new(NonNull::new(wl_surface)?),
        ));
    }

    None
}

#[cfg(target_os = "windows")]
fn surface_to_raw(surface: &gdk::Surface) -> Option<RawWindowHandle> {
    let hwnd = surface
        .downcast_ref::<gdk_win32::Win32Surface>()
        .map(|s| gdk_win32::Win32Surface::handle(s))
        .and_then(|h| std::num::NonZeroIsize::new(h.0.addr() as isize))?;
    Some(RawWindowHandle::Win32(
        raw_window_handle::Win32WindowHandle::new(hwnd),
    ))
}

/// Extract a [`RawDisplayHandle`] from a GDK display.
pub(crate) fn display_to_raw(display: &gdk::Display) -> Option<RawDisplayHandle> {
    let backend = display.backend();
    if backend.is_x11() {
        // GDK4 (unlike GDK3) does not support multiple X screens — the display
        // always has exactly one root window.  screen = 0 is therefore correct
        // for all GDK4 + X11 environments.  In the rare case of a multi-screen
        // X server where surface creation fails, the caller should fall back to
        // gdk_x11_display_get_xdisplay() + XDefaultScreen() from x11-dl.
        Some(RawDisplayHandle::Xlib(
            raw_window_handle::XlibDisplayHandle::new(NonNull::new(display.as_ptr().cast()), 0),
        ))
    } else if backend.is_wayland() {
        Some(RawDisplayHandle::Wayland(
            raw_window_handle::WaylandDisplayHandle::new(NonNull::new(display.as_ptr().cast())?),
        ))
    } else if backend.is_broadway() {
        Some(RawDisplayHandle::Web(
            raw_window_handle::WebDisplayHandle::new(),
        ))
    } else if backend.is_win32() {
        Some(RawDisplayHandle::Windows(
            raw_window_handle::WindowsDisplayHandle::new(),
        ))
    } else if backend.is_macos() {
        Some(RawDisplayHandle::AppKit(
            raw_window_handle::AppKitDisplayHandle::new(),
        ))
    } else {
        None
    }
}
