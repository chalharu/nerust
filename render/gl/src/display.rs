//! Platform display construction shared by the pipeline factory and the tests.
//! Single home for the per-platform `DisplayApiPreference` selection so the
//! test harness depends on this leaf instead of the renderer module.

use glutin::display::{Display, DisplayApiPreference};
use raw_window_handle::RawDisplayHandle;

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
