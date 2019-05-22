<!---
 Copyright (c) 2018 Mitsuharu Seki

 This Source Code Form is subject to the terms of the Mozilla Public
 License, v. 2.0. If a copy of the MPL was not distributed with this
 file, You can obtain one at http://mozilla.org/MPL/2.0/.
-->

# Nerust

An NES emulator written in Rust

## Usage

### Non GTK+ (glutin) Version

#### Dependencies

Cargo
Rust

#### Build

cargo build -p nerust_glutin --release

#### Run

target/release/nerust [Rom File Path]

### Non GTK+ Version

#### Dependencies

Cargo
Rust
GTK+3 v3.16 or greater.

#### Build

cargo build -p nerust_gtk --release

#### Run

target/release/nerust_gtk

## Licence

[MPL-2.0](https://github.com/chalharu/Nerust/blob/master/LICENSE)

## Author

[Mitsuharu Seki](https://github.com/chalharu)
