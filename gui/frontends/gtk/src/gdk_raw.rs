use gtk::gdk;
use gtk::prelude::*;
use raw_window_handle::{RawDisplayHandle, RawWindowHandle};

/// Extract a [`RawWindowHandle`] from a GDK surface.
#[cfg(target_os = "macos")]
pub(crate) fn surface_to_raw(surface: &gdk::Surface) -> Option<RawWindowHandle> {
    use std::ffi::c_void;
    use std::ptr::NonNull;
    surface
        .downcast_ref::<gdk_macos::MacosSurface>()
        .and_then(|s| {
            let native_window = s.native().cast(); // GdkMacosWindow* (NSWindow*)
            unsafe { objc2::rc::Retained::<objc2_app_kit::NSWindow>::retain(native_window) }
        })
        .and_then(|ns_window| ns_window.contentView())
        .map(|ns_view| {
            RawWindowHandle::AppKit(raw_window_handle::AppKitWindowHandle::new(
                NonNull::new(objc2::rc::Retained::into_raw(ns_view) as *mut c_void).unwrap(),
            ))
        })
}

/// Extract a [`RawDisplayHandle`] from a GDK display.
#[cfg(target_os = "macos")]
pub(crate) fn display_to_raw(_display: &gdk::Display) -> Option<RawDisplayHandle> {
    Some(RawDisplayHandle::AppKit(
        raw_window_handle::AppKitDisplayHandle::new(),
    ))
}
