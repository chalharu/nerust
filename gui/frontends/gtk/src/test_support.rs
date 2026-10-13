//! Shared gate for GTK widget tests: exactly one process-wide GTK
//! init, skipped cleanly when the environment is hostile.
//!
//! The gate attempts GTK4 init directly: success runs the tests, a
//! real failure (no display, hostile container) skips them. A
//! `/proc/self/maps` scan for legacy toolkits is deliberately NOT
//! used: merely mapping a library never initializes it, and this
//! crate's test closure does not link one. One shared [`OnceLock`]
//! also guarantees at most one init even when the runner shares a
//! process across tests (plain multithreaded `cargo test`).
//!
//! CI note: the coverage-gtk job forces software rendering
//! (`GSK_RENDERER=cairo`, `GDK_BACKEND=x11`) so surface creation
//! does not depend on container GL.

#[cfg(target_os = "macos")]
pub(crate) fn ui_test_available() -> bool {
    // macOS requires main-thread GTK init which the Rust test
    // harness cannot guarantee.
    false
}

#[cfg(not(target_os = "macos"))]
pub(crate) fn ui_test_available() -> bool {
    static GTK_AVAILABLE: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *GTK_AVAILABLE.get_or_init(|| gtk::init().is_ok() && gtk::gdk::Display::default().is_some())
}
