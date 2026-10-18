//! tao frontend: native window + iced (tiny-skia) UI. Modules,
//! inner -> outer: [`event`] loop events, [`convert`] tao<->iced values,
//! [`menu`] native menu, [`aux`] iced aux windows, [`runtime`] event loop,
//! [`app`] facade. [`gtk_embed`] is the Linux/BSD render target.

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
mod menu;
mod ports;
#[cfg(test)]
pub(crate) mod test_dialogs;
#[cfg(test)]
pub(crate) mod test_gpu;
#[cfg(test)]
pub(crate) mod test_menu;
#[cfg(test)]
pub(crate) mod test_opener;
#[cfg(test)]
pub(crate) mod test_session;

pub(crate) mod aux;
pub(crate) mod runtime;

use nerust_gui_shell::context::FrontendContext;
use nerust_run_options::RunOptions;

pub fn run(ctx: FrontendContext, options: RunOptions) {
    nerust_crash_guard::run_guarded(
        || run_inner(ctx, options),
        |report| {
            // rfd spawns an external portal process, so it can still
            // show after the event loop died. stderr stays the log home.
            rfd::MessageDialog::new()
                .set_title("nerust")
                .set_description(format!(
                    "The application crashed: {}\n\nDetails were written to the log (stderr).",
                    report.message(),
                ))
                .set_level(rfd::MessageLevel::Error)
                .set_buttons(rfd::MessageButtons::Ok)
                .show();
        },
    );
}

fn run_inner(ctx: FrontendContext, options: RunOptions) {
    let mut window = match app::Window::new(ctx) {
        Ok(window) => window,
        Err(detail) => {
            log::error!("failed to initialise tao frontend: {detail}");
            rfd::MessageDialog::new()
                .set_title("nerust")
                .set_description(&detail)
                .set_level(rfd::MessageLevel::Error)
                .set_buttons(rfd::MessageButtons::Ok)
                .show();
            return;
        }
    };
    if let Some(path) = options.rom_path
        && !window.load_path(&path)
    {
        log::info!("starting without a ROM (bad --rom path)");
    }
    window.run();
}
