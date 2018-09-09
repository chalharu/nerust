// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

mod apu;
mod cartridge;
pub mod controller;
mod cpu;
mod mirror_mode;
mod ppu;

use self::apu::Core as Apu;
use self::cartridge::Cartridge;
use self::controller::Controller;
use self::cpu::Core as Cpu;
use self::mirror_mode::MirrorMode;
use self::ppu::Core as Ppu;
use failure::Error;

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

pub trait Speaker {
    fn push(&mut self, data: i16);
}

pub struct Console {
    cpu: Cpu,
    ppu: Ppu,
    apu: Apu,
    cartridge: Box<Cartridge>,
    wram: [u8; 2048],
}

impl Console {
    pub fn new<I: Iterator<Item = u8>>(
        input: &mut I,
        sound_sample_rate: f32,
    ) -> Result<Console, Error> {
        Ok(Self {
            cpu: Cpu::new(),
            ppu: Ppu::new(),
            apu: Apu::new(sound_sample_rate),
            cartridge: try!(cartridge::try_from(input)),
            wram: [0; 2048],
        })
    }

    pub fn step<S: Screen, C: Controller, SP: Speaker>(
        &mut self,
        screen: &mut S,
        controller: &mut C,
        speaker: &mut SP,
    ) -> bool {
        let mut result = false;
        self.cpu.step(
            &mut self.ppu,
            &mut self.cartridge,
            controller,
            &mut self.apu,
            &mut self.wram,
        );
        for _ in 0..3 {
            if self.ppu.step(screen, &mut self.cartridge, &mut self.cpu) {
                result = true;
            }
            self.cartridge.step();
        }
        self.apu.step(&mut self.cpu, &mut self.cartridge, speaker);
        result
    }
}
