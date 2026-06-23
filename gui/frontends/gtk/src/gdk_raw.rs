use gtk::gdk;
use gtk::prelude::*;
use raw_window_handle::{
    RawDisplayHandle, RawWindowHandle, WaylandWindowHandle, XlibDisplayHandle,
};
use std::ffi::c_void;
use std::ptr::NonNull;

/// On non-Linux platforms GTK is not available, so raw handle extraction
/// always fails. The cfg is consolidated here rather than scattered.
#[cfg(all(unix, not(target_os = "macos")))]
fn gdk_ptr(ptr: *mut c_void) -> Option<NonNull<c_void>> {
    NonNull::new(ptr)
}

#[cfg(not(all(unix, not(target_os = "macos"))))]
fn gdk_ptr(_ptr: *mut c_void) -> Option<NonNull<c_void>> {
    None
}

/// Extract a [`RawWindowHandle`] from a GDK surface.
pub(crate) fn surface_to_raw(surface: &gdk::Surface) -> Option<RawWindowHandle> {
    let ptr = gdk_ptr(surface.as_ptr() as *mut c_void)?;
    Some(RawWindowHandle::Wayland(WaylandWindowHandle::new(ptr)))
}

/// Extract a [`RawDisplayHandle`] from a GDK display.
pub(crate) fn display_to_raw(display: &gdk::Display) -> Option<RawDisplayHandle> {
    let ptr = gdk_ptr(display.as_ptr() as *mut c_void)?;
    Some(RawDisplayHandle::Xlib(XlibDisplayHandle::new(Some(ptr), 0)))
}
