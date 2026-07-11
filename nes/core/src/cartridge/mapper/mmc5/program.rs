use nerust_input_traits::OpenBusReadResult;

use super::Mmc5;
use crate::{mapper::CartridgeDataDao, mapper_state::MapperStateDao};

#[derive(Clone, Copy)]
pub(super) enum ProgramTarget {
    Rom(usize),
    Ram(usize),
    OpenBus,
}

impl Mmc5 {
    fn prg_ram_writable(&self) -> bool {
        self.prg_ram_protect_1 == 0x02 && self.prg_ram_protect_2 == 0x01
    }

    pub(super) fn read_exram_cpu(&self, address: usize) -> OpenBusReadResult {
        if self.exram_mode >= 2 {
            OpenBusReadResult::new(self.exram[address - 0x5C00], 0xFF)
        } else {
            OpenBusReadResult::new(0, 0)
        }
    }

    pub(super) fn write_exram_cpu(&mut self, address: usize, value: u8) {
        if self.exram_mode == 2 || (self.exram_mode <= 1 && self.in_frame) {
            self.exram[address - 0x5C00] = value;
        }
    }

    pub(super) fn program_target_6000_7fff(&self, cpu_address: usize) -> ProgramTarget {
        self.program_target_from_register(
            self.prg_banks[0] & 0x07,
            true,
            0x2000,
            cpu_address & 0x1FFF,
        )
    }

    pub(super) fn program_target_8000_ffff(&self, cpu_address: usize) -> ProgramTarget {
        let offset = cpu_address - 0x8000;
        match self.prg_mode {
            0 => self.program_target_from_register(self.prg_banks[4], false, 0x8000, offset),
            1 => {
                if cpu_address < 0xC000 {
                    self.program_target_from_register(self.prg_banks[2], true, 0x4000, offset)
                } else {
                    self.program_target_from_register(
                        self.prg_banks[4],
                        false,
                        0x4000,
                        cpu_address - 0xC000,
                    )
                }
            }
            2 => match cpu_address {
                0x8000..=0xBFFF => {
                    self.program_target_from_register(self.prg_banks[2], true, 0x4000, offset)
                }
                0xC000..=0xDFFF => self.program_target_from_register(
                    self.prg_banks[3],
                    true,
                    0x2000,
                    cpu_address - 0xC000,
                ),
                _ => self.program_target_from_register(
                    self.prg_banks[4],
                    false,
                    0x2000,
                    cpu_address - 0xE000,
                ),
            },
            3 => match cpu_address {
                0x8000..=0x9FFF => self.program_target_from_register(
                    self.prg_banks[1],
                    true,
                    0x2000,
                    cpu_address - 0x8000,
                ),
                0xA000..=0xBFFF => self.program_target_from_register(
                    self.prg_banks[2],
                    true,
                    0x2000,
                    cpu_address - 0xA000,
                ),
                0xC000..=0xDFFF => self.program_target_from_register(
                    self.prg_banks[3],
                    true,
                    0x2000,
                    cpu_address - 0xC000,
                ),
                _ => self.program_target_from_register(
                    self.prg_banks[4],
                    false,
                    0x2000,
                    cpu_address - 0xE000,
                ),
            },
            _ => unreachable!(),
        }
    }

    fn program_target_from_register(
        &self,
        register_value: u8,
        allow_ram_toggle: bool,
        window_len: usize,
        offset: usize,
    ) -> ProgramTarget {
        let bank_units = window_len / 0x2000;
        let base_bank = (usize::from(register_value & 0x7F)) & !(bank_units.saturating_sub(1));
        let mapped_offset = base_bank * 0x2000 + offset;
        if allow_ram_toggle && register_value & 0x80 == 0 {
            if self.mapper_state_ref().sram.is_empty() {
                ProgramTarget::OpenBus
            } else {
                ProgramTarget::Ram(mapped_offset % self.mapper_state_ref().sram.len())
            }
        } else if self.data_ref().prog_rom_len() == 0 {
            ProgramTarget::OpenBus
        } else {
            ProgramTarget::Rom(mapped_offset % self.data_ref().prog_rom_len())
        }
    }

    pub(super) fn read_program_target(&self, target: ProgramTarget) -> OpenBusReadResult {
        match target {
            ProgramTarget::Rom(address) => {
                OpenBusReadResult::new(self.data_ref().read_prog_rom(address), 0xFF)
            }
            ProgramTarget::Ram(address) => {
                OpenBusReadResult::new(self.mapper_state_ref().sram[address], 0xFF)
            }
            ProgramTarget::OpenBus => OpenBusReadResult::new(0, 0),
        }
    }

    pub(super) fn write_program_target(&mut self, target: ProgramTarget, value: u8) {
        if self.prg_ram_writable()
            && let ProgramTarget::Ram(address) = target
            && let Some(slot) = self.mapper_state_mut().sram.get_mut(address)
        {
            *slot = value;
        }
    }
}
