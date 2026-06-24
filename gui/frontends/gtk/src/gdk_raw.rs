use std::ffi::c_void;
use std::ptr::NonNull;

use gtk::gdk;
use gtk::prelude::*;
use raw_window_handle::{RawDisplayHandle, RawWindowHandle};

/// Extract a [`RawWindowHandle`] from a GDK surface.
///
/// Uses `surface.display().backend()` to determine the backend at runtime,
/// then extracts the native handle via `gdk4-sys` FFI. This avoids adding
/// `gdk4-x11` / `gdk4-wayland` crate dependencies.
pub(crate) fn surface_to_raw(surface: &gdk::Surface) -> Option<RawWindowHandle> {
    let backend = surface.display().backend();
    let ptr = surface.as_ptr() as *mut c_void;

    #[cfg(all(unix, not(target_os = "macos")))]
    if backend.is_x11() {
        unsafe extern "C" {
            fn gdk_x11_surface_get_xid(surface: *mut c_void) -> u64;
        }
        let xid = unsafe { gdk_x11_surface_get_xid(ptr) };
        return Some(RawWindowHandle::Xlib(
            raw_window_handle::XlibWindowHandle::new(xid),
        ));
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    if backend.is_wayland() {
        unsafe extern "C" {
            fn gdk_wayland_surface_get_wl_surface(surface: *mut c_void) -> *mut c_void;
        }
        let wl_surface = unsafe { gdk_wayland_surface_get_wl_surface(ptr) };
        return Some(RawWindowHandle::Wayland(
            raw_window_handle::WaylandWindowHandle::new(NonNull::new(wl_surface)?),
        ));
    }

    #[cfg(target_os = "macos")]
    if backend.is_macos() {
        return surface
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
            });
    }

    None
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
