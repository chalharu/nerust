use super::{ChrBankSet, Mmc5};
use crate::OpenBusReadResult;
use crate::mapper::CartridgeDataDao;
use crate::mapper_state::MapperStateDao;
use crate::ppu_memory_access::PpuReadAccess;

impl Mmc5 {
    pub(super) fn exram_visible_to_ppu(&self) -> bool {
        self.exram_mode <= 1
    }

    pub(super) fn extended_attributes_enabled(&self) -> bool {
        self.substitutions_enabled && self.exram_mode == 1 && self.current_split_tile.is_none()
    }

    fn split_chr_banks_enabled(&self) -> bool {
        self.substitutions_enabled && self.sprite_size_16
    }

    pub(super) fn fill_attribute_byte(&self) -> u8 {
        let value = self.fill_attribute & 0x03;
        value | (value << 2) | (value << 4) | (value << 6)
    }

    pub(super) fn extended_attribute_byte(&self) -> u8 {
        let palette = (self.exram[self.current_background_tile_index] >> 6) & 0x03;
        palette | (palette << 2) | (palette << 4) | (palette << 6)
    }

    fn extended_attribute_chr_bank(&self) -> usize {
        usize::from(self.exram[self.current_background_tile_index] & 0x3F)
            | (usize::from(self.chr_upper_bits & 0x03) << 6)
    }

    pub(super) fn nametable_table_and_offset(address: usize) -> (usize, usize) {
        let address = 0x2000 | (address & 0x0FFF);
        ((address >> 10) & 0x03, address & 0x03FF)
    }

    fn chr_storage_len(&self) -> usize {
        if self.data_ref().char_rom_len() > 0 {
            self.data_ref().char_rom_len()
        } else {
            self.mapper_state_ref().vram.len()
        }
    }

    fn read_chr_storage(&self, address: usize) -> u8 {
        if self.data_ref().char_rom_len() > 0 {
            self.data_ref()
                .read_char_rom(address % self.data_ref().char_rom_len())
        } else if self.mapper_state_ref().vram.is_empty() {
            0
        } else {
            self.mapper_state_ref().vram[address % self.mapper_state_ref().vram.len()]
        }
    }

    fn write_chr_storage(&mut self, address: usize, value: u8) {
        if self.data_ref().char_rom_len() == 0 && !self.mapper_state_ref().vram.is_empty() {
            let len = self.mapper_state_ref().vram.len();
            self.mapper_state_mut().vram[address % len] = value;
        }
    }

    pub(super) fn read_character_with_access(
        &self,
        address: usize,
        access: PpuReadAccess,
    ) -> OpenBusReadResult {
        self.chr_address(address, access).map_or_else(
            || OpenBusReadResult::new((address & 0xFF) as u8, 0xFF),
            |mapped| OpenBusReadResult::new(self.read_chr_storage(mapped), 0xFF),
        )
    }

    pub(super) fn write_character_with_access(
        &mut self,
        address: usize,
        value: u8,
        access: PpuReadAccess,
    ) {
        if let Some(mapped) = self.chr_address(address, access) {
            self.write_chr_storage(mapped, value);
        }
    }

    fn chr_address(&self, address: usize, access: PpuReadAccess) -> Option<usize> {
        let address = address & 0x1FFF;
        let storage_len = self.chr_storage_len();
        if storage_len == 0 {
            return None;
        }

        let mapped = if matches!(access, PpuReadAccess::BackgroundPattern)
            && let Some(split_tile) = self.current_split_tile
        {
            self.split_chr_address(address, split_tile)
        } else if matches!(access, PpuReadAccess::BackgroundPattern)
            && self.extended_attributes_enabled()
        {
            self.extended_attribute_chr_bank() * 0x1000 + (address & 0x0FFF)
        } else {
            let (bank, size) = match self.active_chr_bank_set(access) {
                ChrBankSet::Sprite => self.sprite_chr_bank(address),
                ChrBankSet::Background => self.background_chr_bank(address),
            };
            usize::from(bank) * size + (address & (size - 1))
        };
        Some(mapped % storage_len)
    }

    fn active_chr_bank_set(&self, access: PpuReadAccess) -> ChrBankSet {
        match access {
            PpuReadAccess::BackgroundPattern => {
                if self.split_chr_banks_enabled() {
                    ChrBankSet::Background
                } else {
                    ChrBankSet::Sprite
                }
            }
            PpuReadAccess::CpuData => {
                if self.split_chr_banks_enabled() {
                    self.last_chr_bank_set
                } else {
                    ChrBankSet::Sprite
                }
            }
            PpuReadAccess::SpritePattern
            | PpuReadAccess::BackgroundNameTable
            | PpuReadAccess::BackgroundAttribute => ChrBankSet::Sprite,
        }
    }

    fn sprite_chr_bank(&self, address: usize) -> (u16, usize) {
        let address = address & 0x1FFF;
        match self.chr_mode {
            0 => (self.sprite_chr_banks[7], 0x2000),
            1 => (
                self.sprite_chr_banks[if address < 0x1000 { 3 } else { 7 }],
                0x1000,
            ),
            2 => (
                self.sprite_chr_banks[match (address >> 11) & 0x03 {
                    0 => 1,
                    1 => 3,
                    2 => 5,
                    _ => 7,
                }],
                0x0800,
            ),
            3 => (self.sprite_chr_banks[address >> 10], 0x0400),
            _ => unreachable!(),
        }
    }

    fn background_chr_bank(&self, address: usize) -> (u16, usize) {
        let address = address & 0x1FFF;
        match self.chr_mode {
            0 => (self.background_chr_banks[3], 0x2000),
            1 => (self.background_chr_banks[3], 0x1000),
            2 => (
                self.background_chr_banks[if (address & 0x0800) == 0 { 1 } else { 3 }],
                0x0800,
            ),
            3 => (
                self.background_chr_banks
                    [((address >> 10) & 0x03) % self.background_chr_banks.len()],
                0x0400,
            ),
            _ => unreachable!(),
        }
    }
}
