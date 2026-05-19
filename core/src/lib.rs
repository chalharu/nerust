#![allow(
    dead_code,
    reason = "emulator components include future mapper/APU hooks"
)]

// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

mod apu;
mod cart_device;
mod cartridge;
mod cartridge_data;
mod cartridge_error;
pub mod controller;
mod cpu;
mod mapper;
mod mapper_state;
mod ppu;
mod ppu_bus_event;
mod status;

use self::apu::Core as Apu;
use self::cart_device::Cartridge;
use self::controller::Controller;
use self::cpu::Core as Cpu;
use self::ppu::Core as Ppu;
use nerust_screen_traits::Screen;
use nerust_sound_traits::MixerInput;

pub use self::cartridge_data::{CartridgeData, CartridgeDataParts, RomFormat};
pub use self::cartridge_error::CartridgeError;
pub use self::status::mirror_mode::MirrorMode;

pub type Error = Box<dyn std::error::Error + Send + Sync + 'static>;

#[derive(
    serde_derive::Serialize, serde_derive::Deserialize, Debug, Clone, Copy, PartialEq, Eq, Default,
)]
#[serde(rename_all = "snake_case")]
pub enum Mmc3IrqVariant {
    #[default]
    Sharp,
    Nec,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct CoreOptions {
    pub mmc3_irq_variant: Option<Mmc3IrqVariant>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RomInfo {
    pub format: RomFormat,
    pub mapper_type: u16,
    pub sub_mapper_type: u8,
    pub mirror_mode: MirrorMode,
    pub has_battery: bool,
    pub trainer_len: usize,
    pub prg_rom_len: usize,
    pub chr_rom_len: usize,
    pub prg_ram_len: usize,
    pub save_prg_ram_len: usize,
    pub chr_ram_len: usize,
    pub save_chr_ram_len: usize,
    pub raw_file_len: usize,
    pub body_len: usize,
}

#[derive(serde_derive::Serialize, serde_derive::Deserialize)]
pub struct Core {
    cpu: Cpu,
    ppu: Ppu,
    apu: Apu,
    cartridge: Box<dyn Cartridge>,
}

impl Core {
    pub fn new(cartridge_data: CartridgeData) -> Result<Core, Error> {
        Self::new_with_options(cartridge_data, CoreOptions::default())
    }

    pub fn new_with_options(
        cartridge_data: CartridgeData,
        options: CoreOptions,
    ) -> Result<Core, Error> {
        cartridge_data.validate()?;
        let mut cpu = Cpu::new();
        let cartridge = cartridge::try_from_with_options(cartridge_data, options)?;
        let apu = Apu::new(cpu.interrupt_mut());
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
        self.apu.reset(self.cpu.interrupt_mut());
    }

    pub fn peek_work_ram(&self, address: usize) -> Option<u8> {
        self.cpu.peek_work_ram(address)
    }

    pub fn peek_cartridge_ram(&self, address: usize) -> Option<OpenBusReadResult> {
        if (0x6000..=0x7FFF).contains(&address) {
            Some(self.cartridge.read(address))
        } else {
            None
        }
    }

    pub fn inspect_cartridge(
        cartridge_data: &CartridgeData,
        raw_file_len: usize,
    ) -> Result<RomInfo, Error> {
        Ok(RomInfo {
            format: cartridge_data.format(),
            mapper_type: cartridge_data.mapper_type(),
            sub_mapper_type: cartridge_data.sub_mapper_type(),
            mirror_mode: cartridge_data.mirror_mode(),
            has_battery: cartridge_data.has_battery(),
            trainer_len: cartridge_data.trainer().len(),
            prg_rom_len: cartridge_data.prog_rom_len(),
            chr_rom_len: cartridge_data.char_rom_len(),
            prg_ram_len: cartridge_data.pram_length(),
            save_prg_ram_len: cartridge_data.save_pram_length(),
            chr_ram_len: cartridge_data.vram_length(),
            save_chr_ram_len: cartridge_data.save_vram_length(),
            raw_file_len,
            body_len: raw_file_len.saturating_sub(16),
        })
    }

    pub fn peek_ppu_vram(&self, address: usize) -> Option<u8> {
        self.ppu.peek_vram(address, self.cartridge.mirror_mode())
    }

    pub fn step<S: Screen, M: MixerInput>(
        &mut self,
        screen: &mut S,
        controller: &mut dyn Controller,
        mixer: &mut M,
    ) -> bool {
        self.step_cycle(screen, controller, mixer, mixer.sample_rate())
    }

    pub fn run_frame<S: Screen, M: MixerInput>(
        &mut self,
        screen: &mut S,
        controller: &mut dyn Controller,
        mixer: &mut M,
    ) -> u64 {
        let mut cycles = 0;
        let mixer_sample_rate = mixer.sample_rate();
        loop {
            cycles += 1;
            if self.step_cycle(screen, controller, mixer, mixer_sample_rate) {
                return cycles;
            }
        }
    }

    #[inline(always)]
    fn step_cycle<S: Screen, M: MixerInput>(
        &mut self,
        screen: &mut S,
        controller: &mut dyn Controller,
        mixer: &mut M,
        mixer_sample_rate: u32,
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
                .step(screen, self.cartridge.as_mut(), self.cpu.interrupt_mut())
            {
                result = true;
            }
        }
        self.cartridge.step();
        self.apu.step(&mut self.cpu, mixer, mixer_sample_rate);

        result
    }
}

#[derive(serde_derive::Serialize, serde_derive::Deserialize, Debug, Clone, Copy)]
struct OpenBus {
    data: u8,
}

impl OpenBus {
    pub(crate) fn new() -> Self {
        Self { data: 0 }
    }

    pub(crate) fn unite(&mut self, data: OpenBusReadResult) -> u8 {
        let result = (self.data & !data.mask) | (data.data & data.mask);
        self.data = result;
        result
    }
}

#[derive(Debug, Copy, Clone)]
pub struct OpenBusReadResult {
    pub data: u8,
    pub mask: u8,
}

impl OpenBusReadResult {
    pub fn new(data: u8, mask: u8) -> Self {
        Self { data, mask }
    }
}

#[cfg(test)]
fn nrom_test_cartridge() -> Box<dyn Cartridge> {
    cartridge::try_from(
        CartridgeData::new(CartridgeDataParts {
            format: RomFormat::INes,
            prog_rom: vec![0; 0x8000],
            char_rom: vec![0; 0x2000],
            pram_length: 0,
            save_pram_length: 0,
            vram_length: 0,
            save_vram_length: 0,
            mapper_type: 0,
            mirror_mode: MirrorMode::Horizontal,
            has_battery: false,
            sub_mapper_type: 0,
            trainer: Vec::new(),
        })
        .expect("test cartridge data should be valid"),
    )
    .expect("cartridge should construct")
}
