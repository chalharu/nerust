//! tao frontend: native window + iced (tiny-skia) UI.
//!
//! Module map (inner -> outer):
//! - [`event`]: event-loop user events (leaf: menu and runtime share it).
//! - [`convert`]: tao<->iced value conversions (keys, modifiers, events).
//! - [`menu`]: native application menu (muda) and its sync snapshot.
//! - [`aux`]: iced-based auxiliary windows (bridge, debugger, PPU viewer,
//!   settings) on the generic [`aux::shell`] host.
//! - [`runtime`]: tao event loop ([`runtime::WindowRuntime`]), the main
//!   window ([`runtime::HostState`]), and the debug drain.
//! - [`app`]: thin [`app::Window`] facade over the runtime.
//!
//! Platform glue:
//! - [`gtk_embed`]: Linux/BSD render target below the menubar.

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

pub(crate) mod aux;
pub(crate) mod runtime;

use nerust_gui_shell::context::FrontendContext;
use nerust_run_options::RunOptions;

pub fn run(ctx: FrontendContext, options: RunOptions) {
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
    if let Some(path) = options.rom_path {
        let _ = window.load_path(&path);
    }
    window.run();
}
