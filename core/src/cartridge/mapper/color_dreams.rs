// Copyright (c) 2018 chalharu
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use super::Cartridge;
use crate::cartridge_rom::CartridgeData;
use crate::interrupt::Interrupt;
use crate::mapper::{CartridgeDataDao, Mapper};
use crate::mapper_state::{MapperState, MapperStateDao};

#[derive(serde_derive::Serialize, serde_derive::Deserialize)]
pub(crate) struct ColorDreams {
    cartridge_data: CartridgeData,
    state: MapperState,
}

#[typetag::serde]
impl Cartridge for ColorDreams {}

impl ColorDreams {
    pub(crate) fn new(data: CartridgeData) -> Self {
        Self {
            cartridge_data: data,
            state: MapperState::new(),
        }
    }
}

impl CartridgeDataDao for ColorDreams {
    fn data_mut(&mut self) -> &mut CartridgeData {
        &mut self.cartridge_data
    }

    fn data_ref(&self) -> &CartridgeData {
        &self.cartridge_data
    }
}

impl MapperStateDao for ColorDreams {
    fn mapper_state_mut(&mut self) -> &mut MapperState {
        &mut self.state
    }

    fn mapper_state_ref(&self) -> &MapperState {
        &self.state
    }
}

impl Mapper for ColorDreams {
    fn program_page_len(&self) -> usize {
        0x8000
    }

    fn character_page_len(&self) -> usize {
        0x2000
    }

    fn initialize(&mut self) {
        let last_page =
            (self.data_ref().prog_rom_len() / self.program_page_len()).saturating_sub(1);
        self.change_program_page(0, last_page);
        self.change_character_page(0, 0);
    }

    fn name(&self) -> &str {
        "Color Dreams (Mapper11)"
    }

    fn bus_conflicts(&self) -> bool {
        true
    }

    fn write_register(&mut self, _address: usize, value: u8, _interrupt: &mut Interrupt) {
        self.change_program_page(0, usize::from(value & 0x0F));
        self.change_character_page(0, usize::from(value >> 4));
    }
}

#[cfg(test)]
mod tests {
    use super::Cartridge;
    use super::ColorDreams;
    use crate::cartridge_data_parts::CartridgeDataParts;
    use crate::cartridge_rom::CartridgeData;
    use crate::interrupt::Interrupt;
    use crate::mapper::Mapper;
    use nerust_contract_mirror::MirrorMode;
    use nerust_contract_rom::RomFormat;

    fn test_data() -> CartridgeData {
        CartridgeData::new(CartridgeDataParts {
            format: RomFormat::Nes20,
            prog_rom: vec![0; 0x10000],
            char_rom: Vec::new(),
            pram_length: 0,
            save_pram_length: 0,
            vram_length: 0x8000,
            save_vram_length: 0,
            mapper_type: 11,
            mirror_mode: MirrorMode::Vertical,
            has_battery: false,
            sub_mapper_type: 0,
            trainer: Vec::new(),
        })
        .expect("test cartridge data should be valid")
    }

    #[test]
    fn chr_ram_banking_preserves_last_program_bank() {
        let mut mapper = ColorDreams::new(test_data());
        Cartridge::initialize(&mut mapper);
        let mut interrupt = Interrupt::new();

        for bank in (0u8..=0x1F).rev() {
            mapper.write_register(0x8000, (bank << 4) | 0x0F, &mut interrupt);
            for offset in 0..8usize {
                Cartridge::write_character(
                    &mut mapper,
                    0x01FC + (offset * 0x0400),
                    bank << 3 | u8::try_from(offset).unwrap(),
                );
            }
        }

        mapper.write_register(0x8000, 0xFF, &mut interrupt);
        for offset in 0..8usize {
            assert_eq!(
                Cartridge::read_character(&mapper, 0x01FC + (offset * 0x0400)).data,
                0x18 | u8::try_from(offset).unwrap()
            );
        }
    }
}
