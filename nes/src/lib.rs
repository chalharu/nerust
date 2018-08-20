// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

extern crate std as core;

#[macro_use]
extern crate serde_derive;

extern crate serde;
extern crate serde_bytes;

#[macro_use]
extern crate log;

#[macro_use]
extern crate failure;

mod apu;
mod cartridge;
mod controller;
mod cpu;
mod ppu;

use apu::Core as Apu;
use cartridge::Cartridge;
use cpu::Core as Cpu;
use failure::Error;
use ppu::Core as Ppu;
use controller::Controller;

pub struct Console<'a> {
    cpu: Option<Cpu<'a>>,
    ppu: Option<Ppu>,
    cartridge: Box<Cartridge>,
    wram: Option<[u8; 2048]>,
}

impl<'a> Console<'a> {
    pub fn new<I: Iterator<Item = u8>>(input: &mut I) -> Result<Console, Error> {
        Ok(Self {
            cpu: Some(Cpu::new()),
            ppu: Some(Ppu::new()),
            cartridge: try!(cartridge::try_from(input)),
            wram: Some([0; 2048]),
        })
    }
    pub fn step(&mut self) {}
}

#[derive(Serialize, Deserialize, Debug, Copy, Clone)]
pub enum MirrorMode {
    Horizontal,
    Vertical,
    Single0,
    Single1,
    Four,
}

impl MirrorMode {
    fn try_from<'a>(mode: u8) -> Result<MirrorMode, &'a str> {
        match mode {
            0 => Ok(MirrorMode::Horizontal),
            1 => Ok(MirrorMode::Vertical),
            2 => Ok(MirrorMode::Single0),
            3 => Ok(MirrorMode::Single1),
            4 => Ok(MirrorMode::Four),
            _ => Err("parse error"),
        }
    }
}
