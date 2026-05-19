// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use super::CartridgeData;
use crate::OpenBusReadResult;
use crate::cart_device::Cartridge;
use crate::cpu::interrupt::Interrupt;
use crate::mapper::{CartridgeDataDao, Mapper};
use crate::mapper_state::{MapperState, MapperStateDao};
use crate::ppu_memory_access::PpuReadAccess;

#[derive(Debug, Clone, Copy, serde_derive::Serialize, serde_derive::Deserialize, PartialEq, Eq)]
enum ChrBankSet {
    Sprite,
    Background,
}

#[derive(serde_derive::Serialize, serde_derive::Deserialize)]
pub(crate) struct Mmc5 {
    cartridge_data: CartridgeData,
    state: MapperState,
    prg_mode: u8,
    chr_mode: u8,
    prg_ram_protect_1: u8,
    prg_ram_protect_2: u8,
    exram_mode: u8,
    nametable_mapping: [u8; 4],
    fill_tile: u8,
    fill_attribute: u8,
    prg_banks: [u8; 5],
    sprite_chr_banks: [u16; 8],
    background_chr_banks: [u16; 4],
    chr_upper_bits: u8,
    sprite_size_16: bool,
    substitutions_enabled: bool,
    last_chr_bank_set: ChrBankSet,
    current_background_tile_index: usize,
    exram: Vec<u8>,
}

#[typetag::serde]
impl Cartridge for Mmc5 {
    fn read_character(&self, address: usize) -> OpenBusReadResult {
        self.read_character_with_access(address, PpuReadAccess::CpuData)
    }

    fn write_character(&mut self, address: usize, value: u8) {
        self.write_character_with_access(address, value, PpuReadAccess::CpuData);
    }

    fn read_ram(&self, address: usize) -> OpenBusReadResult {
        self.read_program_target(self.program_target_6000_7fff(address + 0x6000))
    }

    fn write_ram(&mut self, address: usize, value: u8, _interrupt: &mut Interrupt) {
        self.write_program_target(self.program_target_6000_7fff(address + 0x6000), value);
    }

    fn read_program(&self, address: usize) -> OpenBusReadResult {
        self.read_program_target(self.program_target_8000_ffff(address + 0x8000))
    }

    fn write_program(&mut self, address: usize, value: u8, _interrupt: &mut Interrupt) {
        let cpu_address = address + 0x8000;
        self.write_program_target(self.program_target_8000_ffff(cpu_address), value);
    }

    fn notify_ppu_ctrl(&mut self, value: u8) {
        self.sprite_size_16 = value & 0x20 != 0;
    }

    fn notify_ppu_mask(&mut self, value: u8) {
        self.substitutions_enabled = value & 0x18 != 0;
    }

    fn read_ppu_pattern(
        &mut self,
        address: usize,
        access: PpuReadAccess,
        _interrupt: &mut Interrupt,
    ) -> OpenBusReadResult {
        self.read_character_with_access(address, access)
    }

    fn write_ppu_pattern(&mut self, address: usize, value: u8, _interrupt: &mut Interrupt) {
        self.write_character_with_access(address, value, PpuReadAccess::CpuData);
    }

    fn read_ppu_nametable(
        &mut self,
        address: usize,
        access: PpuReadAccess,
        ciram: &mut [u8],
    ) -> OpenBusReadResult {
        let (table, offset) = Self::nametable_table_and_offset(address);
        if matches!(access, PpuReadAccess::BackgroundNameTable) {
            self.current_background_tile_index = offset & 0x03FF;
        }
        if matches!(access, PpuReadAccess::BackgroundAttribute)
            && self.extended_attributes_enabled()
        {
            return OpenBusReadResult::new(self.extended_attribute_byte(), 0xFF);
        }

        match self.nametable_mapping[table] {
            0 | 1 => OpenBusReadResult::new(
                ciram[(usize::from(self.nametable_mapping[table] & 0x01) << 10) | offset],
                0xFF,
            ),
            2 => {
                if self.exram_visible_to_ppu() {
                    OpenBusReadResult::new(self.exram[offset], 0xFF)
                } else {
                    OpenBusReadResult::new(0, 0xFF)
                }
            }
            3 => OpenBusReadResult::new(
                if offset >= 0x03C0 {
                    self.fill_attribute_byte()
                } else {
                    self.fill_tile
                },
                0xFF,
            ),
            _ => unreachable!(),
        }
    }

    fn write_ppu_nametable(
        &mut self,
        address: usize,
        value: u8,
        ciram: &mut [u8],
        _interrupt: &mut Interrupt,
    ) {
        let (table, offset) = Self::nametable_table_and_offset(address);
        match self.nametable_mapping[table] {
            0 | 1 => {
                let page = usize::from(self.nametable_mapping[table] & 0x01);
                ciram[(page << 10) | offset] = value;
            }
            2 if self.exram_visible_to_ppu() => self.exram[offset] = value,
            _ => {}
        }
    }

    fn peek_ppu_nametable(&self, address: usize, ciram: &[u8]) -> Option<u8> {
        let (table, offset) = Self::nametable_table_and_offset(address);
        Some(match self.nametable_mapping[table] {
            0 | 1 => {
                let page = usize::from(self.nametable_mapping[table] & 0x01);
                ciram[(page << 10) | offset]
            }
            2 => {
                if self.exram_visible_to_ppu() {
                    self.exram[offset]
                } else {
                    0
                }
            }
            3 => {
                if offset >= 0x03C0 {
                    self.fill_attribute_byte()
                } else {
                    self.fill_tile
                }
            }
            _ => unreachable!(),
        })
    }
}

impl Mmc5 {
    pub(crate) fn new(data: CartridgeData) -> Self {
        Self {
            cartridge_data: data,
            state: MapperState::new(),
            prg_mode: 3,
            chr_mode: 0,
            prg_ram_protect_1: 0x03,
            prg_ram_protect_2: 0x03,
            exram_mode: 0x03,
            nametable_mapping: [0, 0, 1, 1],
            fill_tile: 0,
            fill_attribute: 0,
            prg_banks: [0, 0, 0, 0, 0xFF],
            sprite_chr_banks: [0; 8],
            background_chr_banks: [0; 4],
            chr_upper_bits: 0,
            sprite_size_16: false,
            substitutions_enabled: false,
            last_chr_bank_set: ChrBankSet::Sprite,
            current_background_tile_index: 0,
            exram: vec![0; 0x400],
        }
    }

    fn expand_chr_bank(&self, value: u8) -> u16 {
        u16::from(value) | (u16::from(self.chr_upper_bits & 0x03) << 8)
    }

    fn exram_visible_to_ppu(&self) -> bool {
        self.exram_mode <= 1
    }

    fn extended_attributes_enabled(&self) -> bool {
        self.substitutions_enabled && self.exram_mode == 1
    }

    fn split_chr_banks_enabled(&self) -> bool {
        self.substitutions_enabled && self.sprite_size_16
    }

    fn fill_attribute_byte(&self) -> u8 {
        let value = self.fill_attribute & 0x03;
        value | (value << 2) | (value << 4) | (value << 6)
    }

    fn extended_attribute_byte(&self) -> u8 {
        let palette = (self.exram[self.current_background_tile_index] >> 6) & 0x03;
        palette | (palette << 2) | (palette << 4) | (palette << 6)
    }

    fn extended_attribute_chr_bank(&self) -> usize {
        usize::from(self.exram[self.current_background_tile_index] & 0x3F)
            | (usize::from(self.chr_upper_bits & 0x03) << 6)
    }

    fn nametable_table_and_offset(address: usize) -> (usize, usize) {
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

    fn read_character_with_access(
        &self,
        address: usize,
        access: PpuReadAccess,
    ) -> OpenBusReadResult {
        self.chr_address(address, access).map_or_else(
            || OpenBusReadResult::new((address & 0xFF) as u8, 0xFF),
            |mapped| OpenBusReadResult::new(self.read_chr_storage(mapped), 0xFF),
        )
    }

    fn write_character_with_access(&mut self, address: usize, value: u8, access: PpuReadAccess) {
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

    fn prg_ram_writable(&self) -> bool {
        self.prg_ram_protect_1 == 0x02 && self.prg_ram_protect_2 == 0x01
    }

    fn read_exram_cpu(&self, address: usize) -> OpenBusReadResult {
        if self.exram_mode >= 2 {
            OpenBusReadResult::new(self.exram[address - 0x5C00], 0xFF)
        } else {
            OpenBusReadResult::new(0, 0)
        }
    }

    fn write_exram_cpu(&mut self, address: usize, value: u8) {
        // Modes 0/1 only expose ExRAM to the CPU during blanking, which we do not model.
        // Keep CPU writes strict instead of accepting writes that hardware would reject.
        if self.exram_mode == 2 {
            self.exram[address - 0x5C00] = value;
        }
    }

    fn program_target_6000_7fff(&self, cpu_address: usize) -> ProgramTarget {
        self.program_target_from_register(
            self.prg_banks[0] & 0x07,
            true,
            0x2000,
            cpu_address & 0x1FFF,
        )
    }

    fn program_target_8000_ffff(&self, cpu_address: usize) -> ProgramTarget {
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

    fn read_program_target(&self, target: ProgramTarget) -> OpenBusReadResult {
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

    fn write_program_target(&mut self, target: ProgramTarget, value: u8) {
        if self.prg_ram_writable()
            && let ProgramTarget::Ram(address) = target
            && let Some(slot) = self.mapper_state_mut().sram.get_mut(address)
        {
            *slot = value;
        }
    }
}

impl CartridgeDataDao for Mmc5 {
    fn data_mut(&mut self) -> &mut CartridgeData {
        &mut self.cartridge_data
    }

    fn data_ref(&self) -> &CartridgeData {
        &self.cartridge_data
    }
}

impl MapperStateDao for Mmc5 {
    fn mapper_state_mut(&mut self) -> &mut MapperState {
        &mut self.state
    }

    fn mapper_state_ref(&self) -> &MapperState {
        &self.state
    }
}

impl Mapper for Mmc5 {
    fn program_page_len(&self) -> usize {
        0x2000
    }

    fn character_page_len(&self) -> usize {
        0x0400
    }

    fn ram_len_default(&self) -> usize {
        0x10000
    }

    fn initialize(&mut self) {
        self.set_mirror_mode(match self.data_ref().mirror_mode() {
            crate::MirrorMode::Vertical => crate::MirrorMode::Vertical,
            crate::MirrorMode::Horizontal => crate::MirrorMode::Horizontal,
            mode => mode,
        });
    }

    fn name(&self) -> &str {
        "MMC5 (Mapper5)"
    }

    fn read_expansion(&self, address: usize) -> OpenBusReadResult {
        match address {
            0x5C00..=0x5FFF => self.read_exram_cpu(address),
            _ => OpenBusReadResult::new(0, 0),
        }
    }

    fn write_expansion(&mut self, address: usize, value: u8, _interrupt: &mut Interrupt) {
        match address {
            0x5100 => self.prg_mode = value & 0x03,
            0x5101 => self.chr_mode = value & 0x03,
            0x5102 => self.prg_ram_protect_1 = value & 0x03,
            0x5103 => self.prg_ram_protect_2 = value & 0x03,
            0x5104 => self.exram_mode = value & 0x03,
            0x5105 => {
                self.nametable_mapping = [
                    value & 0x03,
                    (value >> 2) & 0x03,
                    (value >> 4) & 0x03,
                    (value >> 6) & 0x03,
                ];
            }
            0x5106 => self.fill_tile = value,
            0x5107 => self.fill_attribute = value & 0x03,
            0x5113..=0x5117 => self.prg_banks[address - 0x5113] = value,
            0x5120..=0x5127 => {
                self.sprite_chr_banks[address - 0x5120] = self.expand_chr_bank(value);
                self.last_chr_bank_set = ChrBankSet::Sprite;
            }
            0x5128..=0x512B => {
                self.background_chr_banks[address - 0x5128] = self.expand_chr_bank(value);
                self.last_chr_bank_set = ChrBankSet::Background;
            }
            0x5130 => self.chr_upper_bits = value & 0x03,
            0x5C00..=0x5FFF => self.write_exram_cpu(address, value),
            _ => {}
        }
    }
}

#[derive(Clone, Copy)]
enum ProgramTarget {
    Rom(usize),
    Ram(usize),
    OpenBus,
}

#[cfg(test)]
mod tests {
    use super::{ChrBankSet, Mmc5};
    use crate::cart_device::Cartridge;
    use crate::cpu::interrupt::Interrupt;
    use crate::mapper::Mapper;
    use crate::ppu::Core as PpuCore;
    use crate::ppu_memory_access::PpuReadAccess;
    use crate::{CartridgeData, CartridgeDataParts, MirrorMode, RomFormat};

    fn test_data() -> CartridgeData {
        CartridgeData::new(CartridgeDataParts {
            format: RomFormat::Nes20,
            prog_rom: (0..0x20000).map(|i| (i / 0x2000) as u8).collect(),
            char_rom: (0..0x40000).map(|i| (i / 0x400) as u8).collect(),
            pram_length: 0,
            save_pram_length: 0,
            vram_length: 0,
            save_vram_length: 0,
            mapper_type: 5,
            mirror_mode: MirrorMode::Horizontal,
            has_battery: false,
            sub_mapper_type: 0,
            trainer: Vec::new(),
        })
        .expect("test cartridge data should be valid")
    }

    #[test]
    fn ppudata_uses_last_written_chr_bank_set_in_split_mode() {
        let mut mapper = Mmc5::new(test_data());
        Cartridge::initialize(&mut mapper);
        mapper.write_expansion(0x5101, 0x03, &mut Interrupt::new());
        mapper.notify_ppu_ctrl(0x20);
        mapper.notify_ppu_mask(0x18);

        mapper.write_expansion(0x5120, 0x02, &mut Interrupt::new());
        mapper.write_expansion(0x5128, 0x07, &mut Interrupt::new());

        assert_eq!(mapper.last_chr_bank_set, ChrBankSet::Background);
        assert_eq!(
            mapper
                .read_ppu_pattern(0x0000, PpuReadAccess::CpuData, &mut Interrupt::new())
                .data,
            0x07
        );
    }

    #[test]
    fn extended_attributes_override_background_palette_and_chr_bank() {
        let mut mapper = Mmc5::new(test_data());
        Cartridge::initialize(&mut mapper);
        mapper.notify_ppu_mask(0x18);
        mapper.write_expansion(0x5104, 0x01, &mut Interrupt::new());
        mapper.write_expansion(0x5130, 0x01, &mut Interrupt::new());
        mapper.exram[0] = 0b10_000011;

        let mut ciram = vec![0; 0x800];
        let _ = mapper.read_ppu_nametable(0x2000, PpuReadAccess::BackgroundNameTable, &mut ciram);

        assert_eq!(
            mapper
                .read_ppu_nametable(0x23C0, PpuReadAccess::BackgroundAttribute, &mut ciram)
                .data,
            0xAA
        );
        assert_eq!(
            mapper
                .read_ppu_pattern(
                    0x0000,
                    PpuReadAccess::BackgroundPattern,
                    &mut Interrupt::new()
                )
                .data,
            0x0C
        );
    }

    #[test]
    fn fill_mode_supplies_nametable_and_attribute_bytes() {
        let mut mapper = Mmc5::new(test_data());
        Cartridge::initialize(&mut mapper);
        Mapper::write_expansion(&mut mapper, 0x5105, 0xFF, &mut Interrupt::new());
        Mapper::write_expansion(&mut mapper, 0x5106, 0x25, &mut Interrupt::new());
        Mapper::write_expansion(&mut mapper, 0x5107, 0x03, &mut Interrupt::new());
        let mut ciram = vec![0; 0x800];

        assert_eq!(
            mapper
                .read_ppu_nametable(0x2000, PpuReadAccess::CpuData, &mut ciram)
                .data,
            0x25
        );
        assert_eq!(
            mapper
                .read_ppu_nametable(0x23C0, PpuReadAccess::CpuData, &mut ciram)
                .data,
            0xFF
        );
    }

    #[test]
    fn exram_can_be_executed_from_expansion_space() {
        let mut mapper = Mmc5::new(test_data());
        Cartridge::initialize(&mut mapper);
        mapper.write_expansion(0x5104, 0x02, &mut Interrupt::new());
        mapper.write_expansion(0x5C00, 0xA9, &mut Interrupt::new());
        mapper.write_expansion(0x5C01, 0x5A, &mut Interrupt::new());

        assert_eq!(mapper.read_expansion(0x5C00).data, 0xA9);
        assert_eq!(mapper.read_expansion(0x5C01).data, 0x5A);
    }

    #[test]
    fn mirrored_ppuctrl_writes_do_not_toggle_sprite_size() {
        let mut mapper = Mmc5::new(test_data());
        Cartridge::initialize(&mut mapper);
        let mut ppu = PpuCore::new();
        let mut interrupt = Interrupt::new();

        ppu.write_register(0x2008, 0x20, &mut mapper, &mut interrupt);
        assert!(!mapper.sprite_size_16);

        ppu.write_register(0x2000, 0x20, &mut mapper, &mut interrupt);
        assert!(mapper.sprite_size_16);
    }

    #[test]
    fn exram_mode_zero_rejects_cpu_writes() {
        let mut mapper = Mmc5::new(test_data());
        Cartridge::initialize(&mut mapper);
        mapper.write_expansion(0x5104, 0x00, &mut Interrupt::new());
        mapper.write_expansion(0x5C00, 0xA9, &mut Interrupt::new());

        assert_eq!(mapper.read_expansion(0x5C00).mask, 0);

        mapper.write_expansion(0x5104, 0x02, &mut Interrupt::new());
        assert_eq!(mapper.read_expansion(0x5C00).data, 0x00);
    }
}
