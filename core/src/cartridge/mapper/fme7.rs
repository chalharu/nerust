// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use super::Cartridge;
use super::mapper_save_api::{
    CartridgeRuntimeState, MAPPER_KIND_FME7, PersistenceError, decode_payload, encode_payload,
};
use crate::CartridgeData;
use crate::cpu::interrupt::{Interrupt, IrqSource};
use crate::mapper::{CartridgeDataDao, Mapper};
use crate::mapper_state::{MapperState, MapperStateDao};
use crate::status::mirror_mode::MirrorMode;

const IRQ_ENABLE: u8 = 0x01;
const IRQ_COUNT: u8 = 0x80;

#[derive(serde_derive::Serialize, serde_derive::Deserialize)]
struct Fme7RuntimeState {
    command: u8,
    chr_banks: [u8; 8],
    prg_banks: [u8; 4],
    irq_control: u8,
    irq_counter: u16,
}

#[derive(serde_derive::Serialize, serde_derive::Deserialize)]
pub(crate) struct Fme7 {
    cartridge_data: CartridgeData,
    state: MapperState,
    command: u8,
    chr_banks: [u8; 8],
    prg_banks: [u8; 4],
    irq_control: u8,
    irq_counter: u16,
}

#[typetag::serde]
impl Cartridge for Fme7 {
    fn export_runtime_state(&self) -> Result<CartridgeRuntimeState, PersistenceError> {
        Ok(CartridgeRuntimeState {
            mapper_state: self.state.clone(),
            extra_kind: MAPPER_KIND_FME7.into(),
            extra_body: encode_payload(&Fme7RuntimeState {
                command: self.command,
                chr_banks: self.chr_banks,
                prg_banks: self.prg_banks,
                irq_control: self.irq_control,
                irq_counter: self.irq_counter,
            })?,
        })
    }

    fn import_runtime_state(
        &mut self,
        state: CartridgeRuntimeState,
    ) -> Result<(), PersistenceError> {
        if state.extra_kind != MAPPER_KIND_FME7 {
            return Err(PersistenceError::Validation(
                "unexpected FME-7 runtime kind".into(),
            ));
        }
        self.state.validate_for_import(
            &state.mapper_state,
            self.data_ref().prog_rom_len(),
            self.data_ref().char_rom_len(),
        )?;
        let runtime: Fme7RuntimeState = decode_payload(&state.extra_body)?;
        self.state = state.mapper_state;
        self.command = runtime.command;
        self.chr_banks = runtime.chr_banks;
        self.prg_banks = runtime.prg_banks;
        self.irq_control = runtime.irq_control;
        self.irq_counter = runtime.irq_counter;
        Ok(())
    }
}

impl Fme7 {
    pub(crate) fn new(data: CartridgeData) -> Self {
        Self {
            cartridge_data: data,
            state: MapperState::new(),
            command: 0,
            chr_banks: [0, 1, 2, 3, 4, 5, 6, 7],
            prg_banks: [3, 0, 1, 2],
            irq_control: 0,
            irq_counter: 0,
        }
    }

    fn update_chr_banks(&mut self) {
        for (index, bank) in self.chr_banks.into_iter().enumerate() {
            self.change_character_page(index, usize::from(bank));
        }
    }

    fn update_prg_banks(&mut self) {
        self.change_program_page(0, usize::from(self.prg_banks[1] & 0x3F));
        self.change_program_page(1, usize::from(self.prg_banks[2] & 0x3F));
        self.change_program_page(2, usize::from(self.prg_banks[3] & 0x3F));
        let last_bank =
            (self.data_ref().prog_rom_len() / self.program_page_len()).saturating_sub(1);
        self.change_program_page(3, last_bank);
    }

    fn ram_bank(&self) -> usize {
        usize::from(self.prg_banks[0] & 0x3F)
    }

    fn ram_bank_count(&self) -> usize {
        self.mapper_state_ref().sram.len() / self.ram_page_len()
    }

    fn ram_base_address(&self, index: usize) -> Option<usize> {
        let bank_count = self.ram_bank_count();
        if bank_count == 0 {
            return None;
        }
        Some((self.ram_bank() % bank_count) * self.ram_page_len() + index)
    }

    fn ram_is_enabled(&self) -> bool {
        (self.prg_banks[0] & 0xC0) == 0xC0
    }

    fn ram_is_open_bus(&self) -> bool {
        (self.prg_banks[0] & 0xC0) == 0x40
    }

    fn ram_is_rom(&self) -> bool {
        !self.ram_is_enabled() && !self.ram_is_open_bus()
    }

    fn read_rom_window_6000(&self, index: usize) -> Option<u8> {
        if !self.ram_is_rom() {
            return None;
        }
        let rom_bank_count = self.data_ref().prog_rom_len() / self.program_page_len();
        if rom_bank_count == 0 {
            return None;
        }
        let bank = self.ram_bank() % rom_bank_count;
        Some(
            self.data_ref()
                .read_prog_rom(bank * self.ram_page_len() + index),
        )
    }

    fn write_command_data(&mut self, value: u8, interrupt: &mut Interrupt) {
        match self.command & 0x0F {
            0x00..=0x07 => {
                self.chr_banks[usize::from(self.command & 0x07)] = value;
                self.update_chr_banks();
            }
            0x08..=0x0B => {
                self.prg_banks[usize::from((self.command & 0x0F) - 0x08)] = value;
                self.update_prg_banks();
            }
            0x0C => {
                self.set_mirror_mode(match value & 0x03 {
                    0 => MirrorMode::Vertical,
                    1 => MirrorMode::Horizontal,
                    2 => MirrorMode::Single0,
                    _ => MirrorMode::Single1,
                });
            }
            0x0D => {
                interrupt.clear_irq(IrqSource::EXTERNAL);
                self.irq_control = value & (IRQ_ENABLE | IRQ_COUNT);
                if (self.irq_control & IRQ_ENABLE) == 0 {
                    interrupt.clear_irq(IrqSource::EXTERNAL);
                }
            }
            0x0E => self.irq_counter = (self.irq_counter & 0xFF00) | u16::from(value),
            0x0F => self.irq_counter = (self.irq_counter & 0x00FF) | (u16::from(value) << 8),
            _ => {}
        }
    }
}

impl CartridgeDataDao for Fme7 {
    fn data_mut(&mut self) -> &mut CartridgeData {
        &mut self.cartridge_data
    }

    fn data_ref(&self) -> &CartridgeData {
        &self.cartridge_data
    }
}

impl MapperStateDao for Fme7 {
    fn mapper_state_mut(&mut self) -> &mut MapperState {
        &mut self.state
    }

    fn mapper_state_ref(&self) -> &MapperState {
        &self.state
    }
}

impl Mapper for Fme7 {
    fn program_page_len(&self) -> usize {
        0x2000
    }

    fn character_page_len(&self) -> usize {
        0x0400
    }

    fn initialize(&mut self) {
        self.update_chr_banks();
        self.update_prg_banks();
        self.change_ram_page(0, 0);
    }

    fn name(&self) -> &str {
        "FME-7 (Mapper69)"
    }

    fn read_ram(&self, index: usize) -> Option<u8> {
        if self.ram_is_enabled() {
            self.ram_base_address(index)
                .and_then(|address| self.mapper_state_ref().sram.get(address).copied())
        } else if self.ram_is_open_bus() {
            Some(self.prg_banks[0])
        } else {
            self.read_rom_window_6000(index)
        }
    }

    fn write_ram(&mut self, index: usize, data: u8) {
        if self.ram_is_enabled()
            && let Some(address) = self.ram_base_address(index)
            && let Some(slot) = self.mapper_state_mut().sram.get_mut(address)
        {
            *slot = data;
        }
    }

    fn write_register(&mut self, address: usize, value: u8, interrupt: &mut Interrupt) {
        match address & 0xE000 {
            0x8000 => self.command = value & 0x0F,
            0xA000 => self.write_command_data(value, interrupt),
            _ => {}
        }
    }

    fn step(&mut self, interrupt: &mut Interrupt) {
        self.step_irq(interrupt);
    }
}

impl Fme7 {
    fn irq_should_fire(&self, previous: u16) -> bool {
        (self.irq_control & (IRQ_COUNT | IRQ_ENABLE)) == (IRQ_COUNT | IRQ_ENABLE) && previous == 0
    }
}

impl Fme7 {
    pub(crate) fn step_irq(&mut self, interrupt: &mut Interrupt) {
        if (self.irq_control & IRQ_COUNT) == 0 {
            return;
        }

        let previous = self.irq_counter;
        self.irq_counter = previous.wrapping_sub(1);
        if self.irq_should_fire(previous) {
            interrupt.set_irq(IrqSource::EXTERNAL);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Cartridge;
    use super::Fme7;
    use crate::CartridgeData;
    use crate::CartridgeDataParts;
    use crate::RomFormat;
    use crate::cpu::interrupt::{Interrupt, IrqSource};
    use crate::mapper::Mapper;
    use crate::status::mirror_mode::MirrorMode;

    fn test_data() -> CartridgeData {
        CartridgeData::new(CartridgeDataParts {
            format: RomFormat::Nes20,
            prog_rom: (0..0x20000).map(|i| (i / 0x2000) as u8).collect(),
            char_rom: vec![0; 0x2000],
            pram_length: 0x8000,
            save_pram_length: 0,
            vram_length: 0,
            save_vram_length: 0,
            mapper_type: 69,
            mirror_mode: MirrorMode::Horizontal,
            has_battery: false,
            sub_mapper_type: 0,
            trainer: Vec::new(),
        })
        .expect("test cartridge data should be valid")
    }

    #[test]
    fn reg_8_selects_ram_bank_and_rom_mode() {
        let mut mapper = Fme7::new(test_data());
        Cartridge::initialize(&mut mapper);
        let mut interrupt = Interrupt::new();

        mapper.write_register(0x8000, 0x08, &mut interrupt);
        mapper.write_register(0xA000, 0xC2, &mut interrupt);
        Mapper::write_ram(&mut mapper, 0x0000, 0x6B);
        assert_eq!(Mapper::read_ram(&mapper, 0x0000), Some(0x6B));

        mapper.write_register(0xA000, 0x02, &mut interrupt);
        assert_eq!(Mapper::read_ram(&mapper, 0x0000), Some(0x02));
    }

    #[test]
    fn reg_8_wraps_ram_banks_to_available_wram() {
        let mut mapper = Fme7::new(test_data());
        Cartridge::initialize(&mut mapper);
        let mut interrupt = Interrupt::new();

        mapper.write_register(0x8000, 0x08, &mut interrupt);
        for bank in (0u8..=0x0F).rev() {
            mapper.write_register(0xA000, 0xC0 | bank, &mut interrupt);
            Mapper::write_ram(&mut mapper, 0x0900, 0xC0 | bank);
        }

        for bank in 0u8..=0x07 {
            mapper.write_register(0xA000, 0xC0 | bank, &mut interrupt);
            assert_eq!(
                Mapper::read_ram(&mapper, 0x0900),
                Some(0xC0 | (bank & 0x03))
            );
        }
    }

    #[test]
    fn irq_acknowledge_is_only_on_reg_0d_writes() {
        let mut mapper = Fme7::new(test_data());
        Cartridge::initialize(&mut mapper);
        let mut interrupt = Interrupt::new();

        mapper.write_register(0x8000, 0x0D, &mut interrupt);
        mapper.write_register(0xA000, 0x81, &mut interrupt);
        mapper.write_register(0x8000, 0x0E, &mut interrupt);
        mapper.write_register(0xA000, 0x00, &mut interrupt);
        mapper.write_register(0x8000, 0x0F, &mut interrupt);
        mapper.write_register(0xA000, 0x00, &mut interrupt);

        mapper.irq_counter = 0;
        mapper.step_irq(&mut interrupt);
        assert!(interrupt.get_irq(IrqSource::EXTERNAL));

        mapper.write_register(0x8000, 0x0E, &mut interrupt);
        mapper.write_register(0xA000, 0xFF, &mut interrupt);
        assert!(interrupt.get_irq(IrqSource::EXTERNAL));

        mapper.write_register(0x8000, 0x0D, &mut interrupt);
        mapper.write_register(0xA000, 0x81, &mut interrupt);
        assert!(!interrupt.get_irq(IrqSource::EXTERNAL));
    }
}
