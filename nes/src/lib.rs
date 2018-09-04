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
mod mirror_mode;
mod ppu;

use apu::Core as Apu;
use cartridge::Cartridge;
use controller::Controller;
use cpu::Core as Cpu;
use failure::Error;
use mirror_mode::MirrorMode;
use ppu::Core as Ppu;

pub struct RGB {
    pub red: u8,
    pub green: u8,
    pub blue: u8,
}

impl From<u32> for RGB {
    fn from(value: u32) -> RGB {
        RGB {
            red: ((value >> 16) & 0xFF) as u8,
            green: ((value >> 8) & 0xFF) as u8,
            blue: (value & 0xFF) as u8,
        }
    }
}

pub trait Screen {
    fn set_rgb(&mut self, x: u16, y: u16, color: RGB);
}

pub struct Console {
    cpu: Cpu,
    ppu: Ppu,
    apu: Apu,
    cartridge: Box<Cartridge>,
    wram: [u8; 2048],
    controller: Controller,
}

impl Console {
    pub fn new<I: Iterator<Item = u8>>(input: &mut I) -> Result<Console, Error> {
        Ok(Self {
            cpu: Cpu::new(),
            ppu: Ppu::new(),
            apu: Apu::new(),
            cartridge: try!(cartridge::try_from(input)),
            wram: [0; 2048],
            controller: Controller::new(),
        })
    }

    pub fn step<S: Screen>(&mut self, screen: &mut S) -> bool {
        let mut result = false;
        self.cpu.step(
            &mut self.ppu,
            &mut self.cartridge,
            &mut self.controller,
            &mut self.apu,
            &mut self.wram,
        );
        for _ in 0..3 {
            if self.ppu.step(screen, &mut self.cartridge, &mut self.cpu) {
                result = true;
            }
            self.cartridge.step();
        }
        self.apu.step();
        result
    }
}
