use super::Cartridge;
use crate::cartridge_rom::CartridgeData;
use crate::interrupt::Interrupt;
use crate::mapper::{CartridgeDataDao, Mapper};
use crate::mapper_state::{MapperState, MapperStateDao};

#[derive(serde::Serialize, serde::Deserialize)]
pub(crate) struct CrazyClimber {
    cartridge_data: CartridgeData,
    state: MapperState,
}

#[typetag::serde]
impl Cartridge for CrazyClimber {}

impl CrazyClimber {
    pub(crate) fn new(data: CartridgeData) -> Self {
        Self {
            cartridge_data: data,
            state: MapperState::new(),
        }
    }
}

impl CartridgeDataDao for CrazyClimber {
    fn data_mut(&mut self) -> &mut CartridgeData {
        &mut self.cartridge_data
    }

    fn data_ref(&self) -> &CartridgeData {
        &self.cartridge_data
    }
}

impl MapperStateDao for CrazyClimber {
    fn mapper_state_mut(&mut self) -> &mut MapperState {
        &mut self.state
    }

    fn mapper_state_ref(&self) -> &MapperState {
        &self.state
    }
}

impl Mapper for CrazyClimber {
    fn program_page_len(&self) -> usize {
        0x4000
    }

    fn character_page_len(&self) -> usize {
        0x2000
    }

    fn initialize(&mut self) {
        self.change_program_page(0, 0);
        self.change_program_page(1, 0);
        self.change_character_page(0, 0);
    }

    fn name(&self) -> &str {
        "Crazy Climber UNROM (Mapper180)"
    }

    fn bus_conflicts(&self) -> bool {
        true
    }

    fn write_register(&mut self, _address: usize, value: u8, _interrupt: &mut Interrupt) {
        self.change_program_page(1, usize::from(value));
    }
}
