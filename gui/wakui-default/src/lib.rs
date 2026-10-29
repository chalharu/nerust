//! Platform-default backend selection for the `wakui` facade.
//!
//! Depending on this crate alone links exactly one backend: gtk on
//! Linux/BSD, tao on Windows/macOS. Other platforms (including Android,
//! which uses its own `nerust_android` frontend) select a backend
//! explicitly and never depend on this crate.

#[cfg(any(
    target_os = "linux",
    target_os = "dragonfly",
    target_os = "freebsd",
    target_os = "netbsd",
    target_os = "openbsd"
))]
pub use wakui_gtk::{run, show_message_dialog};

#[cfg(any(target_os = "windows", target_os = "macos"))]
pub use wakui_tao::{run, show_message_dialog};
