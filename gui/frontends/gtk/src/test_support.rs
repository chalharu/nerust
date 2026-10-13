//! Shared gate for GTK widget tests: exactly one process-wide GTK
//! init, skipped cleanly when the environment is hostile.
//!
//! Only an *initialized* legacy toolkit conflicts with GTK4: merely
//! mapping `libgtk-3.so` (it appears in every process that links it,
//! initialized or not) is harmless, and this crate's test closure
//! does not even link it. So the gate attempts GTK4 init directly
//! instead of scanning `/proc/self/maps`: success runs the tests, a
//! real failure (no display, hostile container) skips them. One
//! shared [`OnceLock`] also guarantees at most one init even when
//! the runner shares a process across tests (plain multithreaded
//! `cargo test`).

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
