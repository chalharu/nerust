// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use clap::{Arg, Command};
use log::LevelFilter;
use nerust_core::{CoreOptions, Mmc3IrqVariant};
use nerust_sound_openal::prepare_macos_runtime;
use nerust_wgpu::Window;
use simple_logger::SimpleLogger;
use std::fs::File;
use std::io::{BufReader, Read};

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
        .arg(Arg::new("filename").help("Rom file name").required(true))
        .arg(
            Arg::new("mmc3-irq-variant")
                .long("mmc3-irq-variant")
                .value_parser(["sharp", "nec"])
                .help("Override mapper 4 MMC3 IRQ behavior"),
        );

    let matches = app.get_matches();
    let core_options = CoreOptions {
        mmc3_irq_variant: matches
            .get_one::<String>("mmc3-irq-variant")
            .map(|variant| match variant.as_str() {
                "sharp" => Mmc3IrqVariant::Sharp,
                "nec" => Mmc3IrqVariant::Nec,
                _ => unreachable!(),
            }),
    };

    if let Some(mut f) = matches
        .get_one::<String>("filename")
        .and_then(|f| File::open(f).ok())
        .map(BufReader::new)
    {
        let mut buf = Vec::new();
        let _ = f.read_to_end(&mut buf).unwrap();
        let mut window = Window::new();
        window.load_with_options(buf, core_options);
        window.run();
    }
}
