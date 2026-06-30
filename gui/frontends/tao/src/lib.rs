mod app_menu;
mod settings;
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
