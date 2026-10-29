//! `wakui`: feature-gated facade over [`wakui_core`] plus one platform backend.
//!
//! Library users who only need the framework vocabulary depend on
//! `wakui_core` directly and never touch this crate. Depending on `wakui`
//! additionally links exactly one backend and re-exports its `run` and
//! `show_message_dialog` entry points, so the composition root stays free
//! of per-flavor branches:
//!
//! - `wakui` with default features runs the platform-default backend
//!   (`wakui_default`: gtk on Linux/BSD, tao on Windows/macOS).
//! - `wakui/gtk` or `wakui/tao` selects a backend explicitly.
//!
//! Enabling both backends is rejected below; custom backends depend on
//! `wakui_core` only.

#[cfg(all(feature = "wakui_gtk", feature = "wakui_tao"))]
compile_error!(
    "Features 'wakui_gtk' and 'wakui_tao' are mutually exclusive. Enable only one backend."
);

#[cfg(all(
    feature = "wakui_default",
    not(feature = "wakui_gtk"),
    not(feature = "wakui_tao")
))]
pub use wakui_default::{run, show_message_dialog};

#[cfg(all(feature = "wakui_gtk", not(feature = "wakui_tao")))]
pub use wakui_gtk::{run, show_message_dialog};

#[cfg(all(feature = "wakui_tao", not(feature = "wakui_gtk")))]
pub use wakui_tao::{run, show_message_dialog};

pub use wakui_core::*;
