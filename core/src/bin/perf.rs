// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

#[cfg(not(feature = "rom-tooling"))]
fn main() {
    eprintln!("perf requires the `rom-tooling` feature");
    std::process::exit(1);
}

#[cfg(feature = "rom-tooling")]
#[cfg_attr(
    test,
    allow(
        dead_code,
        reason = "binary unit tests compile the perf implementation without invoking the CLI entry point"
    )
)]
#[path = "perf_impl.rs"]
mod perf_impl;

#[cfg(feature = "rom-tooling")]
fn main() {
    perf_impl::main();
}
