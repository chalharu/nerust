//! Shared gate for GTK widget tests: exactly one process-wide GTK
//! init, skipped cleanly when the environment is hostile.
//!
//! Background: in CI, GTK4 init aborts with SIGTRAP (verified
//! 2026-10-13: dropping the maps probe reintroduced the abort in
//! the coverage-gtk job), so widget tests must probe
//! `/proc/self/maps` *before* touching GTK. Merely mapping the
//! legacy library should be harmless in theory, but empirically the
//! abort correlates with its presence there, so the gate stays
//! conservative: skip instead of risking the whole job. One shared
//! [`OnceLock`] also guarantees at most one init even when the
//! runner shares a process across tests (plain multithreaded
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
    *GTK_AVAILABLE.get_or_init(|| {
        if has_legacy_gtk() {
            return false;
        }
        gtk::init().is_ok() && gtk::gdk::Display::default().is_some()
    })
}

#[cfg(all(not(target_os = "macos"), not(target_os = "linux")))]
fn has_legacy_gtk() -> bool {
    false
}

#[cfg(target_os = "linux")]
fn has_legacy_gtk() -> bool {
    // On Linux, check /proc/self/maps for GTK 2/3 libraries that
    // correlate with the GTK4 gtk_init() SIGTRAP abort in CI.
    std::fs::read_to_string("/proc/self/maps")
        .map(|maps| {
            maps.lines()
                .any(|line| line.contains("libgtk-3.so") || line.contains("libgtk-2"))
        })
        .unwrap_or(false)
}
