// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use super::Cartridge;
use super::mapper_save_api::{
    CartridgeRuntimeState, MAPPER_KIND_MMC2, PersistenceError, decode_payload, encode_payload,
};
use crate::CartridgeData;
use crate::cpu::interrupt::Interrupt;
use crate::mapper::{CartridgeDataDao, Mapper};
use crate::mapper_state::{MapperState, MapperStateDao};
use crate::ppu_bus_event::PpuBusEvent;
use crate::status::mirror_mode::MirrorMode;

#[derive(serde_derive::Serialize, serde_derive::Deserialize, Clone, Copy, PartialEq, Eq)]
enum Model {
    Mmc2,
    Mmc4,
}

#[derive(Debug, serde_derive::Serialize, serde_derive::Deserialize, Clone, Copy, PartialEq, Eq)]
enum LatchState {
    Fd,
    Fe,
}

#[derive(serde_derive::Serialize, serde_derive::Deserialize)]
pub(crate) struct Mmc2 {
    cartridge_data: CartridgeData,
    state: MapperState,
    model: Model,
    chr_bank_0_fd: u8,
    chr_bank_0_fe: u8,
    chr_bank_1_fd: u8,
    chr_bank_1_fe: u8,
    latch_0: LatchState,
    latch_1: LatchState,
}

#[derive(serde_derive::Serialize, serde_derive::Deserialize)]
struct Mmc2RuntimeState {
    chr_bank_0_fd: u8,
    chr_bank_0_fe: u8,
    chr_bank_1_fd: u8,
    chr_bank_1_fe: u8,
    latch_0: LatchState,
    latch_1: LatchState,
}

#[typetag::serde]
impl Cartridge for Mmc2 {
    fn export_runtime_state(&self) -> Result<CartridgeRuntimeState, PersistenceError> {
        Ok(CartridgeRuntimeState {
            mapper_state: self.state.clone(),
            extra_kind: MAPPER_KIND_MMC2.into(),
            extra_body: encode_payload(&Mmc2RuntimeState {
                chr_bank_0_fd: self.chr_bank_0_fd,
                chr_bank_0_fe: self.chr_bank_0_fe,
                chr_bank_1_fd: self.chr_bank_1_fd,
                chr_bank_1_fe: self.chr_bank_1_fe,
                latch_0: self.latch_0,
                latch_1: self.latch_1,
            })?,
        })
    }

    fn import_runtime_state(
        &mut self,
        state: CartridgeRuntimeState,
    ) -> Result<(), PersistenceError> {
        if state.extra_kind != MAPPER_KIND_MMC2 {
            return Err(PersistenceError::Validation(
                "unexpected MMC2 runtime kind".into(),
            ));
        }
        self.state.validate_for_import(
            &state.mapper_state,
            self.data_ref().prog_rom_len(),
            self.data_ref().char_rom_len(),
        )?;
        let runtime: Mmc2RuntimeState = decode_payload(&state.extra_body)?;
        self.state = state.mapper_state;
        self.chr_bank_0_fd = runtime.chr_bank_0_fd;
        self.chr_bank_0_fe = runtime.chr_bank_0_fe;
        self.chr_bank_1_fd = runtime.chr_bank_1_fd;
        self.chr_bank_1_fe = runtime.chr_bank_1_fe;
        self.latch_0 = runtime.latch_0;
        self.latch_1 = runtime.latch_1;
        Ok(())
    }
}

impl Mmc2 {
    pub(crate) fn new_mapper9(data: CartridgeData) -> Self {
        Self::new(data, Model::Mmc2)
    }

    pub(crate) fn new_mapper10(data: CartridgeData) -> Self {
        Self::new(data, Model::Mmc4)
    }

    fn new(data: CartridgeData, model: Model) -> Self {
        Self {
            cartridge_data: data,
            state: MapperState::new(),
            model,
            chr_bank_0_fd: 0,
            chr_bank_0_fe: 0,
            chr_bank_1_fd: 0,
            chr_bank_1_fe: 0,
            latch_0: LatchState::Fd,
            latch_1: LatchState::Fd,
        }
    }

    fn update_prg_banks(&mut self, bank: u8) {
        let last_8k_bank = (self.data_ref().prog_rom_len() / 0x2000).saturating_sub(1);
        match self.model {
            Model::Mmc2 => {
                self.change_program_page(0, usize::from(bank));
                self.change_program_page(1, last_8k_bank.saturating_sub(2));
                self.change_program_page(2, last_8k_bank.saturating_sub(1));
                self.change_program_page(3, last_8k_bank);
            }
            Model::Mmc4 => {
                let bank = usize::from(bank) << 1;
                self.change_program_page(0, bank);
                self.change_program_page(1, bank + 1);
                self.change_program_page(2, last_8k_bank.saturating_sub(1));
                self.change_program_page(3, last_8k_bank);
            }
        }
    }

    fn update_chr_banks(&mut self) {
        self.change_character_page(
            0,
            usize::from(match self.latch_0 {
                LatchState::Fd => self.chr_bank_0_fd,
                LatchState::Fe => self.chr_bank_0_fe,
            }),
        );
        self.change_character_page(
            1,
            usize::from(match self.latch_1 {
                LatchState::Fd => self.chr_bank_1_fd,
                LatchState::Fe => self.chr_bank_1_fe,
            }),
        );
    }

    fn set_latches_for_address(&mut self, address: usize) {
        let new_latch_0 = match address & 0x1FFF {
            0x0FD8 => Some(LatchState::Fd),
            0x0FE8..=0x0FEF => Some(LatchState::Fe),
            _ => None,
        };
        let new_latch_1 = match address & 0x1FFF {
            0x1FD8 => Some(LatchState::Fd),
            0x1FE8..=0x1FEF => Some(LatchState::Fe),
            _ => None,
        };

        let mut changed = false;
        if let Some(latch) = new_latch_0
            && self.latch_0 != latch
        {
            self.latch_0 = latch;
            changed = true;
        }
        if let Some(latch) = new_latch_1
            && self.latch_1 != latch
        {
            self.latch_1 = latch;
            changed = true;
        }
        if changed {
            self.update_chr_banks();
        }
    }
}

impl CartridgeDataDao for Mmc2 {
    fn data_mut(&mut self) -> &mut CartridgeData {
        &mut self.cartridge_data
    }

    fn data_ref(&self) -> &CartridgeData {
        &self.cartridge_data
    }
}

impl MapperStateDao for Mmc2 {
    fn mapper_state_mut(&mut self) -> &mut MapperState {
        &mut self.state
    }

    fn mapper_state_ref(&self) -> &MapperState {
        &self.state
    }
}

impl Mapper for Mmc2 {
    fn program_page_len(&self) -> usize {
        0x2000
    }

    fn character_page_len(&self) -> usize {
        0x1000
    }

    fn ram_len_default(&self) -> usize {
        match self.model {
            Model::Mmc2 => 0,
            Model::Mmc4 => 0x2000,
        }
    }

    fn initialize(&mut self) {
        self.update_prg_banks(0);
        self.update_chr_banks();
        self.change_ram_page(0, 0);
    }

    fn name(&self) -> &str {
        match self.model {
            Model::Mmc2 => "MMC2 (Mapper9)",
            Model::Mmc4 => "MMC4 (Mapper10)",
        }
    }

    fn write_register(&mut self, address: usize, value: u8, _interrupt: &mut Interrupt) {
        match address & 0xF000 {
            0xA000 => self.update_prg_banks(value),
            0xB000 => {
                self.chr_bank_0_fd = value;
                self.update_chr_banks();
            }
            0xC000 => {
                self.chr_bank_0_fe = value;
                self.update_chr_banks();
            }
            0xD000 => {
                self.chr_bank_1_fd = value;
                self.update_chr_banks();
            }
            0xE000 => {
                self.chr_bank_1_fe = value;
                self.update_chr_banks();
            }
            0xF000 => self.set_mirror_mode(if value & 1 == 0 {
                MirrorMode::Vertical
            } else {
                MirrorMode::Horizontal
            }),
            _ => {}
        }
    }

    fn notify_ppu_bus_event(&mut self, event: PpuBusEvent, _interrupt: &mut Interrupt) {
        let PpuBusEvent::AddressBusUpdate {
            address,
            from_cpu_register,
            ..
        } = event;
        if !from_cpu_register {
            self.set_latches_for_address(address);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Cartridge;
    use super::{LatchState, Mmc2};
    use crate::CartridgeData;
    use crate::CartridgeDataParts;
    use crate::RomFormat;
    use crate::cpu::interrupt::Interrupt;
    use crate::mapper::Mapper;
    use crate::ppu_bus_event::PpuBusEvent;
    use crate::status::mirror_mode::MirrorMode;

    fn test_data(mapper_type: u16) -> CartridgeData {
        CartridgeData::new(CartridgeDataParts {
            format: RomFormat::Nes20,
            prog_rom: vec![0; 0x20000],
            char_rom: vec![0; 0x20000],
            pram_length: 0,
            save_pram_length: 0,
            vram_length: 0,
            save_vram_length: 0,
            mapper_type,
            mirror_mode: MirrorMode::Horizontal,
            has_battery: false,
            sub_mapper_type: 0,
            trainer: Vec::new(),
        })
        .expect("test cartridge data should be valid")
    }

    #[test]
    fn ppu_fetches_toggle_chr_latches() {
        let mut mapper = Mmc2::new_mapper9(test_data(9));
        Cartridge::initialize(&mut mapper);
        let mut interrupt = Interrupt::new();

        mapper.write_register(0xB000, 2, &mut interrupt);
        mapper.write_register(0xC000, 3, &mut interrupt);
        mapper.write_register(0xD000, 4, &mut interrupt);
        mapper.write_register(0xE000, 5, &mut interrupt);

        mapper.notify_ppu_bus_event(
            PpuBusEvent::AddressBusUpdate {
                address: 0x0FE8,
                ppu_tick: 0,
                from_cpu_register: false,
                access: crate::ppu_bus_event::PpuBusAccess::Read,
            },
            &mut interrupt,
        );
        mapper.notify_ppu_bus_event(
            PpuBusEvent::AddressBusUpdate {
                address: 0x1FE8,
                ppu_tick: 1,
                from_cpu_register: false,
                access: crate::ppu_bus_event::PpuBusAccess::Read,
            },
            &mut interrupt,
        );

        assert_eq!(mapper.latch_0, LatchState::Fe);
        assert_eq!(mapper.latch_1, LatchState::Fe);
        assert_eq!(mapper.character_address(0x0000), Some(3 * 0x1000));
        assert_eq!(mapper.character_address(0x1000), Some(5 * 0x1000));

        mapper.notify_ppu_bus_event(
            PpuBusEvent::AddressBusUpdate {
                address: 0x0FD8,
                ppu_tick: 2,
                from_cpu_register: false,
                access: crate::ppu_bus_event::PpuBusAccess::Read,
            },
            &mut interrupt,
        );

        assert_eq!(mapper.latch_0, LatchState::Fd);
        assert_eq!(mapper.character_address(0x0000), Some(2 * 0x1000));
    }
}
