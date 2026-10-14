//! Shared gate for GTK widget tests: one process-wide init,
//! skipped cleanly when the environment maps legacy GTK.

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
    // On Linux, check /proc/self/maps for GTK 2/3 libraries: their
    // presence correlates with the GTK4 gtk_init() SIGTRAP abort.
    std::fs::read_to_string("/proc/self/maps")
        .map(|maps| {
            maps.lines()
                .any(|line| line.contains("libgtk-3.so") || line.contains("libgtk-2"))
        })
        .unwrap_or(false)
}
