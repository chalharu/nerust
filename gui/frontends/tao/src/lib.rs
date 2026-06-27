mod app_menu;
mod settings;
pub mod settings_window;
mod tao_conversions;
pub mod window;

use std::rc::Rc;

use nerust_screen_video::{GpuFactory, RunOptions};

pub fn run(factory: Box<dyn GpuFactory>, options: RunOptions) {
    let factory: Rc<dyn GpuFactory> = Rc::from(factory);

    let window_options =
        options
            .mmc3_irq_variant
            .as_deref()
            .map(|variant| window::WindowLoadOptions {
                mmc3_irq_variant: Some(match variant {
                    "sharp" => window::WindowMmc3IrqVariant::Sharp,
                    "nec" => window::WindowMmc3IrqVariant::Nec,
                    _ => unreachable!(),
                }),
            });

    let mut window = window::Window::new(factory, window_options);
    if let Some(path) = options.rom_path {
        let _ = window.load_path(&path);
    }
    window.run();
}
