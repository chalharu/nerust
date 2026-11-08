//! Platform display construction shared by the pipeline factory and the tests.
//! Single home for the per-platform `DisplayApiPreference` selection so the
//! test harness depends on this leaf instead of the renderer module.

use glutin::display::{Display, DisplayApiPreference};
use raw_window_handle::{RawDisplayHandle, RawWindowHandle};

/// Open a glutin display for a raw platform handle.
/// Exactly one preference arm compiles per platform.
pub(crate) fn create_display(dh: RawDisplayHandle) -> Result<Display, glutin::error::Error> {
    // Single cfg-selected expression: exactly one arm compiles per
    // platform, so no dead Glx/Wgl/Egl shadow survives.
    #[cfg(target_os = "macos")]
    let preference = DisplayApiPreference::Cgl;
    #[cfg(all(target_os = "windows", not(target_family = "wasm")))]
    let preference = DisplayApiPreference::EglThenWgl(None);
    #[cfg(target_os = "android")]
    let preference = DisplayApiPreference::Egl;
    #[cfg(all(
        unix,
        not(target_os = "macos"),
        not(target_os = "android"),
        not(target_family = "wasm"),
        not(target_os = "ios")
    ))]
    let preference = DisplayApiPreference::EglThenGlx(Box::new(|_reg| {}));
    #[cfg(not(any(
        target_os = "macos",
        target_os = "windows",
        target_os = "android",
        all(
            unix,
            not(target_os = "macos"),
            not(target_os = "android"),
            not(target_family = "wasm"),
            not(target_os = "ios")
        )
    )))]
    let preference = DisplayApiPreference::Egl;

    unsafe { glutin::display::Display::new(dh, preference) }
}

/// Align a toolkit-owned AppKit view with its window backing scale before
/// the GL drawable is created. Toolkits leave `contentsScale` stale on
/// layer-backed content views; the CGL drawable then mismatches the window
/// and readback yields blank pixels without any GL error. Best-effort:
/// a non-AppKit handle, a view without a window, or a layerless view
/// simply has nothing to align, and attach proceeds as before.
#[cfg(target_os = "macos")]
pub(crate) fn normalize_window(window_handle: RawWindowHandle) {
    use objc2::rc::Retained;
    use objc2_app_kit::NSView;
    let RawWindowHandle::AppKit(handle) = window_handle else {
        return;
    };
    let Some(view) = (unsafe { Retained::<NSView>::retain(handle.ns_view.as_ptr().cast()) }) else {
        return;
    };
    let Some(window) = view.window() else { return };
    let scale = window.backingScaleFactor();
    if let Some(layer) = view.layer() {
        layer.setContentsScale(scale);
    }
}

/// Other platforms create correct drawables from the raw handles alone.
#[cfg(not(target_os = "macos"))]
pub(crate) fn normalize_window(_window_handle: RawWindowHandle) {}

#[cfg(all(test, not(target_os = "macos")))]
mod tests {
    #[test]
    fn normalize_window_ignores_non_appkit_handles() {
        // No-op path: never touches GL, safe without a display.
        super::normalize_window(raw_window_handle::RawWindowHandle::Xlib(
            raw_window_handle::XlibWindowHandle::new(1),
        ));
    }
}
