//! tao frontend: native window + iced (tiny-skia) UI.
//! [`event`] loop events, [`convert`] values, [`menu`], [`aux`], [`runtime`], [`app`].

mod app;
mod convert;
mod event;
#[cfg(any(
    target_os = "linux",
    target_os = "dragonfly",
    target_os = "freebsd",
    target_os = "netbsd",
    target_os = "openbsd"
))]
mod gtk_embed;
mod keyboard;
mod menu;
mod ports;

pub(crate) mod aux;
pub(crate) mod runtime;

#[cfg(test)]
pub(crate) mod test_dialogs;
#[cfg(test)]
pub(crate) mod test_menu;
#[cfg(test)]
pub(crate) mod test_opener;
#[cfg(test)]
pub(crate) mod test_session;
#[cfg(test)]
pub(crate) mod testkit;

use nerust_frontend_host::bootstrap::Boot;

/// Exit status for the binary wrapper: the library never terminates the process.
/// Crash containment and boot failure dialogs live in the composition root;
/// see `nerust::run`.
pub fn run(boot: Boot, rom_path: Option<std::path::PathBuf>) -> i32 {
    run_inner(boot, rom_path);
    0
}

fn run_inner(boot: Boot, rom_path: Option<std::path::PathBuf>) {
    let mut window = app::Window::new(boot);
    if let Some(path) = rom_path
        && !window.load_path(&path)
    {
        log::info!("starting without a ROM (bad --rom path)");
    }
    window.run();
}
