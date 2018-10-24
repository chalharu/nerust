// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

mod apu;
mod cartridge;
pub mod controller;
mod cpu;
mod interface;
mod ppu;
mod status;

use self::apu::Core as Apu;
use self::cartridge::Cartridge;
use self::controller::Controller;
use self::cpu::Core as Cpu;
pub use self::interface::*;
use self::ppu::Core as Ppu;
use self::status::mirror_mode::MirrorMode;
use failure::Error;

pub struct Console {
    cpu: Cpu,
    ppu: Ppu,
    apu: Apu,
    cartridge: Box<Cartridge>,
}

impl Console {
    pub fn new<I: Iterator<Item = u8>>(
        input: &mut I,
        sound_sample_rate: u32,
    ) -> Result<Console, Error> {
        Ok(Self {
            cpu: Cpu::new(),
            ppu: Ppu::new(),
            apu: Apu::new(sound_sample_rate),
            cartridge: cartridge::try_from(input)?,
        })
    }

    pub fn reset(&mut self) {
        self.cpu.reset();
        self.ppu.reset();
    }

    pub fn step<S: Screen, SP: Speaker>(
        &mut self,
        screen: &mut S,
        controller: &mut Controller,
        speaker: &mut SP,
    ) -> bool {
        // 1CPUサイクルにつき、APUは1、PPUはNTSC=>3,PAL=>3.2となる
        let mut result = false;
        self.cpu.step(
            &mut self.ppu,
            &mut self.cartridge,
            controller,
            &mut self.apu,
        );
        for _ in 0..3 {
            if self
                .ppu
                .step(screen, &mut self.cartridge, &mut self.cpu.interrupt)
            {
                result = true;
            }
            self.cartridge.step();
        }
        self.apu.step(&mut self.cpu, &mut self.cartridge, speaker);

        result
    }
}

struct OpenBus {
    data: u8,
}

impl OpenBus {
    pub fn new() -> Self {
        Self { data: 0 }
    }

    pub fn unite(&mut self, data: OpenBusReadResult) -> u8 {
        let result = (self.data & !data.mask) | (data.data & data.mask);
        self.data = result;
        result
    }

    pub fn write(&mut self, data: u8) -> u8 {
        self.data = data;
        data
    }
}

pub struct OpenBusReadResult {
    pub data: u8,
    pub mask: u8,
}

impl OpenBusReadResult {
    pub fn new(data: u8, mask: u8) -> Self {
        Self { data, mask }
    }
}
