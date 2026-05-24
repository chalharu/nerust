// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use super::Cartridge;
use crate::cartridge_data::CartridgeData;
use crate::interrupt::Interrupt;
use crate::mapper::{CartridgeDataDao, Mapper};
use crate::mapper_state::{MapperState, MapperStateDao};
use crate::persistence::{
    CartridgeRuntimeState, MAPPER_KIND_ACTION53, PersistenceError, decode_payload, encode_payload,
};
use nerust_contract::MirrorMode;

#[derive(serde_derive::Serialize, serde_derive::Deserialize)]
struct Action53RuntimeState {
    selected_register: u8,
    chr_bank: u8,
    inner_bank: u8,
    mode: u8,
    outer_bank: u8,
    onescreen_select: u8,
}

#[derive(serde_derive::Serialize, serde_derive::Deserialize)]
pub(crate) struct Action53 {
    cartridge_data: CartridgeData,
    state: MapperState,
    selected_register: u8,
    chr_bank: u8,
    inner_bank: u8,
    mode: u8,
    outer_bank: u8,
    onescreen_select: u8,
}

#[typetag::serde]
impl Cartridge for Action53 {
    fn export_runtime_state(&self) -> Result<CartridgeRuntimeState, PersistenceError> {
        Ok(CartridgeRuntimeState {
            mapper_state: self.state.clone(),
            extra_kind: MAPPER_KIND_ACTION53.into(),
            extra_body: encode_payload(&Action53RuntimeState {
                selected_register: self.selected_register,
                chr_bank: self.chr_bank,
                inner_bank: self.inner_bank,
                mode: self.mode,
                outer_bank: self.outer_bank,
                onescreen_select: self.onescreen_select,
            })?,
        })
    }

    fn import_runtime_state(
        &mut self,
        state: CartridgeRuntimeState,
    ) -> Result<(), PersistenceError> {
        if state.extra_kind != MAPPER_KIND_ACTION53 {
            return Err(PersistenceError::Validation(
                "unexpected Action53 runtime kind".into(),
            ));
        }
        self.state
            .validate_for_import(
                &state.mapper_state,
                self.data_ref().prog_rom_len(),
                self.data_ref().char_rom_len(),
            )
            .map_err(PersistenceError::Validation)?;
        let runtime: Action53RuntimeState = decode_payload(&state.extra_body)?;
        self.state = state.mapper_state;
        self.selected_register = runtime.selected_register;
        self.chr_bank = runtime.chr_bank;
        self.inner_bank = runtime.inner_bank;
        self.mode = runtime.mode;
        self.outer_bank = runtime.outer_bank;
        self.onescreen_select = runtime.onescreen_select;
        Ok(())
    }
}

impl Action53 {
    pub(crate) fn new(data: CartridgeData) -> Self {
        Self {
            cartridge_data: data,
            state: MapperState::new(),
            selected_register: 0,
            chr_bank: 0,
            inner_bank: 0,
            mode: 0,
            outer_bank: 0x3F,
            onescreen_select: 0,
        }
    }

    fn update_chr_bank(&mut self) {
        self.change_character_page(0, usize::from(self.chr_bank & 0x03));
    }

    fn calc_prg_bank(&self, cpu_a14: usize) -> usize {
        let mut bank_mode = self.mode >> 2;
        let cpu_a14 = u8::try_from(cpu_a14).unwrap();
        let outer_bank = (self.outer_bank << 1) | cpu_a14;
        let mut current_bank = self.inner_bank & 0x0F;

        if ((bank_mode ^ cpu_a14) & 0x03) == 0x02 {
            bank_mode = 0;
        }
        if (bank_mode & 0x02) == 0 {
            current_bank = (current_bank << 1) | cpu_a14;
        }

        let bank_size_mask = [0x01, 0x03, 0x07, 0x0F][usize::from((bank_mode >> 2) & 0x03)];
        usize::from(((current_bank ^ outer_bank) & bank_size_mask) ^ outer_bank)
    }

    fn update_prg_banks(&mut self) {
        self.change_program_page(0, self.calc_prg_bank(0));
        self.change_program_page(1, self.calc_prg_bank(1));
    }

    fn update_mirroring(&mut self) {
        self.set_mirror_mode(match self.mode & 0x03 {
            0 | 1 => {
                if self.onescreen_select == 0 {
                    MirrorMode::Single0
                } else {
                    MirrorMode::Single1
                }
            }
            2 => MirrorMode::Vertical,
            _ => MirrorMode::Horizontal,
        });
    }

    fn write_selected_register(&mut self, value: u8) {
        match self.selected_register {
            0x00 => {
                self.chr_bank = value;
                self.onescreen_select = (value >> 4) & 0x01;
                self.update_chr_bank();
                self.update_mirroring();
            }
            0x01 => {
                self.inner_bank = value & 0x0F;
                self.onescreen_select = (value >> 4) & 0x01;
                self.update_prg_banks();
                self.update_mirroring();
            }
            0x80 => {
                self.mode = value & 0x3F;
                self.onescreen_select = value & 0x01;
                self.update_prg_banks();
                self.update_mirroring();
            }
            0x81 => {
                self.outer_bank = value & 0x3F;
                self.update_prg_banks();
            }
            _ => {}
        }
    }
}

impl CartridgeDataDao for Action53 {
    fn data_mut(&mut self) -> &mut CartridgeData {
        &mut self.cartridge_data
    }

    fn data_ref(&self) -> &CartridgeData {
        &self.cartridge_data
    }
}

impl MapperStateDao for Action53 {
    fn mapper_state_mut(&mut self) -> &mut MapperState {
        &mut self.state
    }

    fn mapper_state_ref(&self) -> &MapperState {
        &self.state
    }
}

impl Mapper for Action53 {
    fn program_page_len(&self) -> usize {
        0x4000
    }

    fn character_page_len(&self) -> usize {
        0x2000
    }

    fn character_ram_page_len_default(&self) -> usize {
        0x8000
    }

    fn initialize(&mut self) {
        self.update_chr_bank();
        self.update_prg_banks();
        self.update_mirroring();
    }

    fn name(&self) -> &str {
        "Action 53 (Mapper28)"
    }

    fn read_expansion(&self, _address: usize) -> crate::OpenBusReadResult {
        crate::OpenBusReadResult::new(0, 0)
    }

    fn write_expansion(&mut self, address: usize, value: u8, _interrupt: &mut Interrupt) {
        if address == 0x5000 {
            self.selected_register = value;
        }
    }

    fn write_register(&mut self, _address: usize, value: u8, _interrupt: &mut Interrupt) {
        self.write_selected_register(value);
    }
}

#[cfg(test)]
mod tests {
    use super::Action53;
    use super::Cartridge;
    use crate::cartridge_data::{CartridgeData, CartridgeDataParts};
    use crate::interrupt::Interrupt;
    use crate::mapper::Mapper;
    use nerust_contract::MirrorMode;
    use nerust_contract::RomFormat;

    fn test_data() -> CartridgeData {
        CartridgeData::new(CartridgeDataParts {
            format: RomFormat::Nes20,
            prog_rom: vec![0; 0x200000],
            char_rom: Vec::new(),
            pram_length: 0,
            save_pram_length: 0,
            vram_length: 0x8000,
            save_vram_length: 0,
            mapper_type: 28,
            mirror_mode: MirrorMode::Horizontal,
            has_battery: false,
            sub_mapper_type: 0,
            trainer: Vec::new(),
        })
        .expect("test cartridge data should be valid")
    }

    #[test]
    fn mode_written_last_controls_one_screen_page() {
        let mut mapper = Action53::new(test_data());
        Cartridge::initialize(&mut mapper);
        let mut interrupt = Interrupt::new();

        mapper.write_expansion(0x5000, 0x00, &mut interrupt);
        mapper.write_register(0x8000, 0x10, &mut interrupt);
        mapper.write_expansion(0x5000, 0x01, &mut interrupt);
        mapper.write_register(0x8000, 0x10, &mut interrupt);
        mapper.write_expansion(0x5000, 0x80, &mut interrupt);
        mapper.write_register(0x8000, 0x3C, &mut interrupt);

        assert_eq!(mapper.mirror_mode(), MirrorMode::Single0);
    }

    #[test]
    fn chr_d4_controls_one_screen_page_after_chr_write() {
        let mut mapper = Action53::new(test_data());
        Cartridge::initialize(&mut mapper);
        let mut interrupt = Interrupt::new();

        mapper.write_expansion(0x5000, 0x80, &mut interrupt);
        mapper.write_register(0x8000, 0x3C, &mut interrupt);
        mapper.write_expansion(0x5000, 0x00, &mut interrupt);
        mapper.write_register(0x8000, 0x10, &mut interrupt);

        assert_eq!(mapper.mirror_mode(), MirrorMode::Single1);
    }

    #[test]
    fn prg_calculation_matches_reference_windows() {
        let mut mapper = Action53::new(test_data());
        Cartridge::initialize(&mut mapper);
        let mut interrupt = Interrupt::new();

        mapper.write_expansion(0x5000, 0x80, &mut interrupt);
        mapper.write_register(0x8000, 0x30, &mut interrupt);
        mapper.write_expansion(0x5000, 0x81, &mut interrupt);
        mapper.write_register(0x8000, 0x15, &mut interrupt);
        mapper.write_expansion(0x5000, 0x01, &mut interrupt);
        mapper.write_register(0x8000, 0x03, &mut interrupt);

        assert_eq!(mapper.program_address(0x0000), Some(0x26 * 0x4000));
        assert_eq!(mapper.program_address(0x4000), Some(0x27 * 0x4000));
    }
}
