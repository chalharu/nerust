// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

#[cfg(not(feature = "rom-tooling"))]
fn main() {
    eprintln!("rom_tool requires the `rom-tooling` feature");
    std::process::exit(1);
}

#[cfg(feature = "rom-tooling")]
#[path = "rom_tool_impl.rs"]
mod rom_tool_impl;

#[cfg(feature = "rom-tooling")]
fn main() {
    rom_tool_impl::main();
}
