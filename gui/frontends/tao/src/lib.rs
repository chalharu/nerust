mod app_menu;
pub(crate) mod debug_window;
#[cfg(any(
    target_os = "linux",
    target_os = "dragonfly",
    target_os = "freebsd",
    target_os = "netbsd",
    target_os = "openbsd"
))]
mod gtk_content;
pub(crate) mod iced_shell;
pub(crate) mod ppu_window;
pub(crate) mod settings;
pub mod settings_window;
mod tao_conversions;
pub mod window;

use nerust_gui_shell::context::FrontendContext;
use nerust_run_options::RunOptions;

pub fn run(ctx: FrontendContext, options: RunOptions) {
    let mut window = window::Window::new(ctx);
    if let Some(path) = options.rom_path {
        let _ = window.load_path(&path);
    }
    window.run();
}
