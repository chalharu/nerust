// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use clap::{Arg, Command};
use nerust_sound_openal::prepare_macos_runtime;
use nerust_wgpu::Window;
use std::fs::File;
use std::io::{BufReader, Read};

fn main() {
    simple_logger::init().unwrap();
    prepare_macos_runtime();

    let app = Command::new(env!("CARGO_PKG_NAME"))
        .version(env!("CARGO_PKG_VERSION"))
        .author(env!("CARGO_PKG_AUTHORS"))
        .about(env!("CARGO_PKG_DESCRIPTION"))
        .arg(Arg::new("filename").help("Rom file name").required(true));

    let matches = app.get_matches();

    if let Some(mut f) = matches
        .get_one::<String>("filename")
        .and_then(|f| File::open(f).ok())
        .map(BufReader::new)
    {
        let mut buf = Vec::new();
        let _ = f.read_to_end(&mut buf).unwrap();
        let mut window = Window::new();
        window.load(buf);
        window.run();
    }
}
