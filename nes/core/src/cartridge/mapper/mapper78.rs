use super::Cartridge;
use crate::cartridge_rom::CartridgeData;
use crate::interrupt::Interrupt;
use crate::mapper::{CartridgeDataDao, Mapper};
use crate::mapper_state::{MapperState, MapperStateDao};
use crate::mirror::MirrorMode;

#[derive(serde::Serialize, serde::Deserialize)]
pub(crate) struct Mapper78 {
    cartridge_data: CartridgeData,
    state: MapperState,
}

#[typetag::serde]
impl Cartridge for Mapper78 {}

impl Mapper78 {
    pub(crate) fn new(data: CartridgeData) -> Self {
        Self {
            cartridge_data: data,
            state: MapperState::new(),
        }
    }

    fn apply_mirroring(&mut self, value: u8) {
        let bit = (value >> 3) & 0x01;
        let mirror = match self.data_ref().sub_mapper_type() {
            3 => {
                if bit == 0 {
                    MirrorMode::Horizontal
                } else {
                    MirrorMode::Vertical
                }
            }
            _ => {
                if bit == 0 {
                    MirrorMode::Single0
                } else {
                    MirrorMode::Single1
                }
            }
        };
        self.set_mirror_mode(mirror);
    }
}

impl CartridgeDataDao for Mapper78 {
    fn data_mut(&mut self) -> &mut CartridgeData {
        &mut self.cartridge_data
    }

    fn data_ref(&self) -> &CartridgeData {
        &self.cartridge_data
    }
}

impl MapperStateDao for Mapper78 {
    fn mapper_state_mut(&mut self) -> &mut MapperState {
        &mut self.state
    }

    fn mapper_state_ref(&self) -> &MapperState {
        &self.state
    }
}

impl Mapper for Mapper78 {
    fn program_page_len(&self) -> usize {
        0x4000
    }

    fn character_page_len(&self) -> usize {
        0x2000
    }

    fn initialize(&mut self) {
        self.change_program_page(0, 0);
        let last_page =
            (self.data_ref().prog_rom_len() / self.program_page_len()).saturating_sub(1);
        self.change_program_page(1, last_page);
        self.change_character_page(0, 0);
        self.apply_mirroring(0);
    }

    fn name(&self) -> &str {
        "Mapper78"
    }

    fn write_register(&mut self, _address: usize, value: u8, _interrupt: &mut Interrupt) {
        self.change_program_page(0, usize::from(value & 0x07));
        self.change_character_page(0, usize::from(value >> 4));
        self.apply_mirroring(value);
    }
}

#[cfg(test)]
mod tests {
    use super::Cartridge;
    use super::Mapper78;
    use crate::cartridge_data_parts::CartridgeDataParts;
    use crate::cartridge_rom::CartridgeData;
    use crate::interrupt::Interrupt;
    use crate::mapper::Mapper;
    use crate::mirror::MirrorMode;
    use crate::rom_format::RomFormat;

    fn test_data(sub_mapper_type: u8) -> CartridgeData {
        CartridgeData::new(CartridgeDataParts {
            format: RomFormat::Nes20,
            prog_rom: vec![0; 0x20000],
            char_rom: vec![0; 0x8000],
            pram_length: 0,
            save_pram_length: 0,
            vram_length: 0,
            save_vram_length: 0,
            mapper_type: 78,
            mirror_mode: MirrorMode::Horizontal,
            has_battery: false,
            sub_mapper_type,
            trainer: Vec::new(),
        })
        .expect("test cartridge data should be valid")
    }

    #[test]
    fn submapper3_uses_holy_diver_mirroring_polarity() {
        let mut mapper = Mapper78::new(test_data(3));
        Cartridge::initialize(&mut mapper);
        let mut interrupt = Interrupt::new();

        mapper.write_register(0x8000, 0x00, &mut interrupt);
        assert_eq!(mapper.mirror_mode(), MirrorMode::Horizontal);

        mapper.write_register(0x8000, 0x08, &mut interrupt);
        assert_eq!(mapper.mirror_mode(), MirrorMode::Vertical);
    }
}
