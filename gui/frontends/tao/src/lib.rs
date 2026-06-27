mod app_menu;
mod settings;
pub mod settings_window;
mod tao_conversions;
pub mod window;

use std::path::PathBuf;

use clap::{Arg, Command};
use log::LevelFilter;
use nerust_sound_openal::prepare_macos_runtime;
use simple_logger::SimpleLogger;

pub fn run() {
    SimpleLogger::new()
        .with_level(LevelFilter::Warn)
        .env()
        .init()
        .unwrap();
    prepare_macos_runtime();

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
    let window_options = window::WindowLoadOptions {
        mmc3_irq_variant: matches.get_one::<String>("mmc3-irq-variant").map(
            |variant| match variant.as_str() {
                "sharp" => window::WindowMmc3IrqVariant::Sharp,
                "nec" => window::WindowMmc3IrqVariant::Nec,
                _ => unreachable!(),
            },
        ),
    };

    let mut window = window::Window::with_load_options(window_options);
    if let Some(filename) = matches.get_one::<String>("filename").map(PathBuf::from) {
        let _ = window.load_path(&filename);
    }
    window.run();
}
