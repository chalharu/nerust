mod app_menu;
mod settings;
pub mod settings_window;
mod tao_conversions;
pub mod window;

use std::path::PathBuf;
use std::rc::Rc;

use clap::{Arg, Command};
use nerust_screen_video::GpuFactory;

pub fn run(factory: Box<dyn GpuFactory>) {
    let factory: Rc<dyn GpuFactory> = Rc::from(factory);

    let app = Command::new(env!("CARGO_PKG_NAME"))
        .version(env!("CARGO_PKG_VERSION"))
        .author(env!("CARGO_PKG_AUTHORS"))
        .about(env!("CARGO_PKG_DESCRIPTION"))
        .arg(Arg::new("filename").help("Rom file name"))
        .arg(
            Arg::new("mmc3-irq-variant")
                .long("mmc3-irq-variant")
                .value_parser(["sharp", "nec"])
                .help("Override mapper 4 MMC3 IRQ behavior"),
        );

    let matches = app.get_matches();
    let window_options = matches
        .get_one::<String>("mmc3-irq-variant")
        .map(|variant| window::WindowLoadOptions {
            mmc3_irq_variant: Some(match variant.as_str() {
                "sharp" => window::WindowMmc3IrqVariant::Sharp,
                "nec" => window::WindowMmc3IrqVariant::Nec,
                _ => unreachable!(),
            }),
        });

    let mut window = window::Window::new(factory, window_options);
    if let Some(filename) = matches.get_one::<String>("filename").map(PathBuf::from) {
        let _ = window.load_path(&filename);
    }
    window.run();
}
