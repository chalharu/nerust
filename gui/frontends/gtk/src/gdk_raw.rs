use std::ptr::NonNull;

use gdk::prelude::DisplayExtManual as _;
use gio::glib::object::ObjectType as _;
use gtk::gdk;
use raw_window_handle::{RawDisplayHandle, RawWindowHandle};

/// Extract a [`RawWindowHandle`] from a GDK surface.
///
/// Uses `surface.display().backend()` to determine the backend at runtime,
/// then extracts the native handle via `gdk4-sys` FFI.
#[cfg(all(not(unix), not(target_os = "windows")))]
pub(crate) fn surface_to_raw(_surface: &gdk::Surface) -> Option<RawWindowHandle> {
    None
}

/// Extract a [`RawWindowHandle`] from a GDK surface.
///
/// Uses `surface.display().backend()` to determine the backend at runtime,
/// then extracts the native handle via `gdk4-sys` FFI.
#[cfg(all(unix, not(target_os = "macos")))]
pub(crate) fn surface_to_raw(surface: &gdk::Surface) -> Option<RawWindowHandle> {
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

/// Extract a [`RawWindowHandle`] from a GDK surface.
#[cfg(target_os = "macos")]
pub(crate) fn surface_to_raw(surface: &gdk::Surface) -> Option<RawWindowHandle> {
    use gio::prelude::Cast;
    use std::ffi::c_void;
    surface
        .downcast_ref::<gdk_macos::MacosSurface>()
        .and_then(|s| {
            let native_window = s.native().cast();
            unsafe { objc2::rc::Retained::<objc2_app_kit::NSWindow>::retain(native_window) }
        })
        .and_then(|ns_window| ns_window.contentView())
        .map(|ns_view| {
            RawWindowHandle::AppKit(raw_window_handle::AppKitWindowHandle::new(
                NonNull::new(objc2::rc::Retained::into_raw(ns_view) as *mut c_void).unwrap(),
            ))
        })
}

/// Extract a [`RawWindowHandle`] from a GDK surface.
#[cfg(target_os = "windows")]
pub(crate) fn surface_to_raw(surface: &gdk::Surface) -> Option<RawWindowHandle> {
    let hwnd = surface
        .downcast_ref::<gdk_win32::Win32Surface>()
        .map(|s| gdk_win32::Win32Surface::handle(s))
        .and_then(|h| std::num::NonZeroIsize::new(h.0.addr() as isize))?;
    return Some(RawWindowHandle::Win32(
        raw_window_handle::Win32WindowHandle::new(hwnd),
    ));
}

/// Extract a [`RawDisplayHandle`] from a GDK display.
pub(crate) fn display_to_raw(display: &gdk::Display) -> Option<RawDisplayHandle> {
    let backend = display.backend();
    if backend.is_x11() {
        Some(RawDisplayHandle::Xlib(
            raw_window_handle::XlibDisplayHandle::new(NonNull::new(display.as_ptr().cast()), 0),
        ))
    } else if backend.is_wayland() {
        Some(RawDisplayHandle::Wayland(
            raw_window_handle::WaylandDisplayHandle::new(
                NonNull::new(display.as_ptr().cast()).unwrap(),
            ),
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
