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
    pub fn new<I: Iterator<Item = u8>>(input: &mut I) -> Result<Console, Error> {
        let mut cpu = Cpu::new();
        let mut cartridge = cartridge::try_from(input)?;
        let apu = Apu::new(&mut cpu.interrupt, cartridge.as_mut());
        Ok(Self {
            cpu,
            ppu: Ppu::new(),
            apu,
            cartridge,
        })
    }

    pub fn reset(&mut self) {
        self.cpu.reset();
        self.ppu.reset();
        self.apu
            .reset(&mut self.cpu.interrupt, self.cartridge.as_mut());
    }

    pub fn step<S: Screen, M: MixerInput>(
        &mut self,
        screen: &mut S,
        controller: &mut Controller,
        mixer: &mut M,
    ) -> bool {
        // 1CPUサイクルにつき、APUは1、PPUはNTSC=>3,PAL=>3.2となる
        let mut result = false;
        self.cpu.step(
            &mut self.ppu,
            self.cartridge.as_mut(),
            controller,
            &mut self.apu,
        );
        for _ in 0..3 {
            if self
                .ppu
                .step(screen, self.cartridge.as_mut(), &mut self.cpu.interrupt)
            {
                result = true;
            }
        }
        self.cartridge.step();
        self.apu.step(&mut self.cpu, self.cartridge.as_mut(), mixer);

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
