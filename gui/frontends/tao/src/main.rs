// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use clap::{Arg, ArgAction, Command};
use log::LevelFilter;
use nerust_sound_openal::prepare_macos_runtime;
use nerust_tao::settings::{HELPER_FLAG, run_settings_helper_from_stdio};
use nerust_tao::window::{Window, WindowLoadOptions, WindowMmc3IrqVariant};
use simple_logger::SimpleLogger;
use std::path::PathBuf;

fn main() {
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
            Arg::new(HELPER_FLAG)
                .long(HELPER_FLAG)
                .hide(true)
                .action(ArgAction::SetTrue),
        )
        .arg(
            Arg::new("mmc3-irq-variant")
                .long("mmc3-irq-variant")
                .value_parser(["sharp", "nec"])
                .help("Override mapper 4 MMC3 IRQ behavior"),
        );

    let matches = app.get_matches();
    if matches.get_flag(HELPER_FLAG) {
        if let Err(error) = run_settings_helper_from_stdio() {
            eprintln!("{error}");
            std::process::exit(1);
        }
        return;
    }
    let window_options = WindowLoadOptions {
        mmc3_irq_variant: matches.get_one::<String>("mmc3-irq-variant").map(
            |variant| match variant.as_str() {
                "sharp" => WindowMmc3IrqVariant::Sharp,
                "nec" => WindowMmc3IrqVariant::Nec,
                _ => unreachable!(),
            },
        ),
    };

    let mut window = Window::with_load_options(window_options);
    if let Some(filename) = matches.get_one::<String>("filename").map(PathBuf::from) {
        let _ = window.load_path(&filename);
    }
    window.run();
}
