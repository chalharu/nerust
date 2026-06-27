use std::path::PathBuf;

use clap::{Arg, Command};
use log::LevelFilter;
use nerust_screen_video::{GpuFactory, RunOptions};
use simple_logger::SimpleLogger;

fn create_factory() -> Box<dyn GpuFactory> {
    #[cfg(feature = "wgpu")]
    return Box::new(nerust_backend_wgpu::WgpuFactory);
    #[cfg(feature = "opengl")]
    return Box::new(nerust_backend_opengl::GlFactory);
    #[cfg(not(any(feature = "wgpu", feature = "opengl")))]
    compile_error!("No backend selected. Enable feature 'wgpu' or 'opengl'.");
}

fn parse_cli_args() -> RunOptions {
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
    RunOptions {
        rom_path: matches.get_one::<String>("filename").map(PathBuf::from),
        mmc3_irq_variant: matches.get_one::<String>("mmc3-irq-variant").cloned(),
    }
}

pub fn run() {
    SimpleLogger::new()
        .with_level(LevelFilter::Warn)
        .env()
        .init()
        .unwrap();

    let options = parse_cli_args();

    #[cfg(feature = "gtk")]
    nerust_gtk::run(create_factory(), options.clone());
    #[cfg(feature = "tao")]
    nerust_tao::run(create_factory(), options);
    #[cfg(not(any(feature = "gtk", feature = "tao")))]
    compile_error!("No frontend selected. Enable feature 'gtk' or 'tao'.");
}
