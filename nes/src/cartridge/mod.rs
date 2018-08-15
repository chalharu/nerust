// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

pub mod error;
pub mod format;
pub mod mapper;

pub trait Cartridge {
    fn read(&self, address: usize) -> u8;
    fn write(&mut self, address: usize, value: u8);
    fn step(&mut self);
}

pub fn try_from<I: Iterator<Item = u8>>(
    input: &mut I,
) -> Result<Box<Cartridge>, error::CartridgeError> {
    mapper::try_from(try!(format::CartridgeData::try_from(input)))
}
