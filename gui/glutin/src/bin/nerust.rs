// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

#[macro_use] // macroを使うのでmacro_useを追記
extern crate clap;

use clap::{App, Arg};
use nerust_glutin::Window;
use simple_logger;
use std::fs::File;
use std::io::{BufReader, Read};

fn main() {
    // log initialize
    simple_logger::init().unwrap();

    let app = App::new(crate_name!())
        .version(crate_version!())
        .author(crate_authors!())
        .about(crate_description!())
        .arg(
            Arg::with_name("filename")
                .help("Rom file name")
                .required(true),
        );

    // 引数を解析
    let matches = app.get_matches();

    // paが指定されていれば値を表示
    if let Some(mut f) = matches
        .value_of("filename")
        .and_then(|f| File::open(f).ok())
        .map(BufReader::new)
    {
        let mut buf = Vec::new();
        f.read_to_end(&mut buf).unwrap();
        let mut window = Window::new();
        window.load(buf);
        window.run();
    }
}
