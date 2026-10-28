//! `wakui_tao`: wakui platform frontend for iced/tao.
//!
//! Framework-only: depends on [`wakui`] plus the iced/tao toolkits.
//! Never on view definitions, sessions, or settings. Native frontends
//! render [`wakui::control::ControlScreen`] snapshots through [`controls`],
//! adapt them to host windows through [`binding`], and map native input
//! through [`keyboard`], [`tao_map`], and [`convert`].
//!
//! A third-party app builds UI with `wakui` plus this crate only:
//! see `examples/backend_smoke.rs`. The entry point is [`run`]; the
//! composition root injects a boxed [`wakui::app::FrontendApp`].

pub mod app;
pub mod backend;
pub mod binding;
pub mod controls;
pub mod convert;
pub mod disabled;
pub mod domain;
pub mod event;
pub mod handler;
pub mod keyboard;
pub mod menu;
pub mod ports;
pub mod tao_map;
pub mod window;

pub(crate) mod aux;
pub(crate) mod runtime;

#[cfg(any(
    target_os = "linux",
    target_os = "dragonfly",
    target_os = "freebsd",
    target_os = "netbsd",
    target_os = "openbsd"
))]
mod gtk_embed;

#[cfg(test)]
pub(crate) mod test_dialogs;
#[cfg(test)]
pub(crate) mod test_menu;
#[cfg(test)]
pub(crate) mod test_opener;
#[cfg(test)]
pub(crate) mod testkit;

pub use backend::{TaoBackend, register};

use wakui::bootstrap::Boot;

/// Exit status for the binary wrapper: the library never terminates the process.
/// Crash containment and boot failure dialogs live in the composition root.
pub fn run<K: Copy + Eq + std::hash::Hash + 'static>(
    boot: Boot<K>,
    rom_path: Option<std::path::PathBuf>,
) -> i32 {
    run_inner(boot, rom_path);
    0
}

/// Standalone blocking message dialog for boot-failure and crash paths.
/// No event loop is running here (or it just died); `rfd` is an internal
/// detail of this flavor, never named by the composition root.
pub fn show_message_dialog(dialog: &wakui::dialog::MessageDialog) {
    rfd::MessageDialog::new()
        .set_title(&dialog.title)
        .set_description(&dialog.description)
        .set_level(match dialog.level {
            wakui::dialog::MessageLevel::Info => rfd::MessageLevel::Info,
            wakui::dialog::MessageLevel::Warning => rfd::MessageLevel::Warning,
            wakui::dialog::MessageLevel::Error => rfd::MessageLevel::Error,
        })
        .set_buttons(match dialog.buttons {
            wakui::dialog::MessageButtons::Ok => rfd::MessageButtons::Ok,
        })
        .show();
}

fn run_inner<K: Copy + Eq + std::hash::Hash + 'static>(
    boot: Boot<K>,
    rom_path: Option<std::path::PathBuf>,
) {
    let mut window = app::Window::new(boot);
    if let Some(path) = rom_path
        && !window.load_path(&path)
    {
        log::info!("starting without a ROM (bad --rom path)");
    }
    window.run();
}
