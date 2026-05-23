// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use super::Cartridge;
use crate::CartridgeData;
use crate::cpu::interrupt::Interrupt;
use crate::mapper::{CartridgeDataDao, Mapper};
use crate::mapper_state::{MapperState, MapperStateDao};

#[derive(serde_derive::Serialize, serde_derive::Deserialize)]
pub(crate) struct GnRom {
    cartridge_data: CartridgeData,
    state: MapperState,
}

#[typetag::serde]
impl Cartridge for GnRom {}

impl GnRom {
    pub(crate) fn new(data: CartridgeData) -> Self {
        Self {
            cartridge_data: data,
            state: MapperState::new(),
        }
    }
}

impl CartridgeDataDao for GnRom {
    fn data_mut(&mut self) -> &mut CartridgeData {
        &mut self.cartridge_data
    }

    fn data_ref(&self) -> &CartridgeData {
        &self.cartridge_data
    }
}

impl MapperStateDao for GnRom {
    fn mapper_state_mut(&mut self) -> &mut MapperState {
        &mut self.state
    }

    fn mapper_state_ref(&self) -> &MapperState {
        &self.state
    }
}

impl Mapper for GnRom {
    fn program_page_len(&self) -> usize {
        0x8000
    }

    fn character_page_len(&self) -> usize {
        0x2000
    }

    fn initialize(&mut self) {
        self.change_program_page(0, 0);
        self.change_character_page(0, 0);
    }

    fn name(&self) -> &str {
        "GNROM (Mapper66)"
    }

    fn bus_conflicts(&self) -> bool {
        true
    }

    fn write_register(&mut self, _address: usize, value: u8, _interrupt: &mut Interrupt) {
        self.change_program_page(0, usize::from(value >> 4));
        self.change_character_page(0, usize::from(value & 0x0F));
    }
}

#[cfg(test)]
mod tests {
    use super::Cartridge;
    use super::GnRom;
    use crate::CartridgeData;
    use crate::CartridgeDataParts;
    use crate::RomFormat;
    use crate::cpu::interrupt::Interrupt;
    use crate::mapper::Mapper;
    use crate::status::mirror_mode::MirrorMode;

    fn test_data() -> CartridgeData {
        CartridgeData::new(CartridgeDataParts {
            format: RomFormat::Nes20,
            prog_rom: vec![0; 0x40000],
            char_rom: vec![0; 0x20000],
            pram_length: 0,
            save_pram_length: 0,
            vram_length: 0,
            save_vram_length: 0,
            mapper_type: 66,
            mirror_mode: MirrorMode::Horizontal,
            has_battery: false,
            sub_mapper_type: 0,
            trainer: Vec::new(),
        })
        .expect("test cartridge data should be valid")
    }

    #[test]
    fn register_write_splits_prg_and_chr_fields() {
        let mut mapper = GnRom::new(test_data());
        Cartridge::initialize(&mut mapper);
        let mut interrupt = Interrupt::new();

        mapper.write_register(0x8000, 0x12, &mut interrupt);

        assert_eq!(mapper.program_address(0x0000), Some(0x8000));
        assert_eq!(mapper.character_address(0x0000), Some(0x02 * 0x2000));
    }
}
