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
mod cartridge;
pub mod controller;
mod cpu;
mod ppu;
mod status;

use self::apu::Core as Apu;
use self::cartridge::Cartridge;
use self::controller::Controller;
use self::cpu::Core as Cpu;
use self::ppu::Core as Ppu;
use nerust_screen_traits::Screen;
use nerust_sound_traits::MixerInput;

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
pub enum RomFormat {
    INes,
    Nes20,
}

impl RomFormat {
    pub const fn label(self) -> &'static str {
        match self {
            Self::INes => "iNES",
            Self::Nes20 => "NES 2.0",
        }
    }
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
    pub fn new<I: Iterator<Item = u8>>(input: &mut I) -> Result<Core, Error> {
        Self::new_with_options(input, CoreOptions::default())
    }

    pub fn new_with_options<I: Iterator<Item = u8>>(
        input: &mut I,
        options: CoreOptions,
    ) -> Result<Core, Error> {
        let mut cpu = Cpu::new();
        let mut cartridge = cartridge::try_from_with_options(input, options)?;
        let apu = Apu::new(cpu.interrupt_mut(), cartridge.as_mut());
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
            .reset(self.cpu.interrupt_mut(), self.cartridge.as_mut());
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

    pub fn inspect_rom(data: &[u8]) -> Result<RomInfo, Error> {
        if data.len() < 16 {
            return Err(cartridge::error::CartridgeError::UnexpectedEof.into());
        }
        if data[0..4] != [0x4E, 0x45, 0x53, 0x1A] {
            return Err(cartridge::error::CartridgeError::DataError.into());
        }

        let format = if (data[7] & 0x0C) == 0x08 {
            RomFormat::Nes20
        } else {
            RomFormat::INes
        };
        let mut input = data.iter().copied();
        let cartridge_data = cartridge::format::CartridgeData::try_from(&mut input)?;

        Ok(RomInfo {
            format,
            mapper_type: cartridge_data.mapper_type(),
            sub_mapper_type: cartridge_data.sub_mapper_type(),
            mirror_mode: cartridge_data.get_mirror_mode(),
            has_battery: cartridge_data.has_battery(),
            trainer_len: cartridge_data.trainer().len(),
            prg_rom_len: cartridge_data.prog_rom_len(),
            chr_rom_len: cartridge_data.char_rom_len(),
            prg_ram_len: cartridge_data.pram_length(),
            save_prg_ram_len: cartridge_data.save_pram_length(),
            chr_ram_len: cartridge_data.vram_length(),
            save_chr_ram_len: cartridge_data.save_vram_length(),
            raw_file_len: data.len(),
            body_len: data.len().saturating_sub(16),
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
        self.apu.step(
            &mut self.cpu,
            self.cartridge.as_mut(),
            mixer,
            mixer_sample_rate,
        );

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
mod tests {
    use super::{Core, MirrorMode, RomFormat};

    #[test]
    fn inspect_rom_reads_ines_metadata() {
        let mut rom = vec![
            0x4E, 0x45, 0x53, 0x1A, 0x02, 0x01, 0x41, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00,
        ];
        rom.resize(16 + 0x8000 + 0x2000, 0);

        let info = Core::inspect_rom(&rom).expect("rom info should parse");

        assert_eq!(info.format, RomFormat::INes);
        assert_eq!(info.mapper_type, 4);
        assert_eq!(info.sub_mapper_type, 0);
        assert_eq!(info.mirror_mode, MirrorMode::Vertical);
        assert!(!info.has_battery);
        assert_eq!(info.trainer_len, 0);
        assert_eq!(info.prg_rom_len, 0x8000);
        assert_eq!(info.chr_rom_len, 0x2000);
        assert_eq!(info.prg_ram_len, 0x2000);
        assert_eq!(info.save_prg_ram_len, 0);
        assert_eq!(info.chr_ram_len, 0);
        assert_eq!(info.save_chr_ram_len, 0);
        assert_eq!(info.raw_file_len, rom.len());
        assert_eq!(info.body_len, rom.len() - 16);
    }
}
