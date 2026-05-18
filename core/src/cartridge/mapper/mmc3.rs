// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

// Mapper 4

use super::super::{CartridgeDataDao, Mapper, MapperState, MapperStateDao};
use super::{Cartridge, CartridgeData};
use crate::cpu::interrupt::{Interrupt, IrqSource};
use crate::{MirrorMode, Mmc3IrqVariant};

// MMC3 clocks on A12 rising edges only after A12 remained low for three falling
// edges of M2, which corresponds to roughly 9 PPU ticks.
const A12_LOW_FILTER_TICKS: u64 = 9;

#[derive(serde_derive::Serialize, serde_derive::Deserialize)]
pub(crate) struct Mmc3 {
    cartridge_data: CartridgeData,
    state: MapperState,
    bank_select: u8,         // $8000-$9FFE, even
    bank_data: [u8; 8],      // $8000-$9FFE, odd
    mirroring: u8,           // $A000-$BFFE, even
    program_ram_protect: u8, // $A001-$BFFF, odd
    irq_latch: u8,           // $C000-$DFFE, even
    irq_reload: bool,        // $C001-$DFFF, odd
    irq_counter: u8,
    irq_enabled: bool, // disable = $E000-$FFFE, even , enable = $E001-$FFFF, odd
    last_a12_high: bool,
    last_a12_low_tick: u64,
}

#[typetag::serde]
impl Cartridge for Mmc3 {}

impl Mmc3 {
    pub(crate) fn new(data: CartridgeData) -> Self {
        Self {
            cartridge_data: data,
            state: MapperState::new(),
            bank_select: 0,
            bank_data: [0, 0, 0, 0, 0, 0, 0, 1],
            mirroring: 0,
            program_ram_protect: 0x80,
            irq_latch: 0,
            irq_reload: false,
            irq_counter: 0,
            irq_enabled: false,
            last_a12_high: false,
            last_a12_low_tick: 0,
        }
    }

    fn clear_ram_mapping(&mut self) {
        for slot in &mut self.mapper_state_mut().sram_page_table {
            *slot = None;
        }
    }

    fn program_bank_count(&self) -> usize {
        self.data_ref().prog_rom_len() / self.program_page_len()
    }

    fn character_bank_count(&self) -> usize {
        if self.mapper_state_ref().character_mapping_mode == crate::cartridge::MappingMode::Ram {
            self.mapper_state_ref().vram.len() / self.character_page_len()
        } else {
            self.data_ref().char_rom_len() / self.character_page_len()
        }
    }

    fn map_program_bank(&mut self, slot: usize, bank: usize) {
        if self.program_bank_count() > 0 {
            self.change_program_page(slot, bank % self.program_bank_count());
        }
    }

    fn map_character_bank(&mut self, slot: usize, bank: usize) {
        if self.character_bank_count() > 0 {
            self.change_character_page(slot, bank % self.character_bank_count());
        }
    }

    fn program_ram_enabled(&self) -> bool {
        if self.is_mmc6() {
            self.mmc6_program_ram_chip_enabled()
        } else {
            !self.mapper_state_ref().sram.is_empty() && (self.program_ram_protect & 0x80) != 0
        }
    }

    fn program_ram_write_enabled(&self) -> bool {
        if self.is_mmc6() {
            self.mmc6_program_ram_chip_enabled()
        } else {
            self.program_ram_enabled() && (self.program_ram_protect & 0x40) == 0
        }
    }

    fn is_mmc6(&self) -> bool {
        self.data_ref().sub_mapper_type() == 1
    }

    fn mmc6_program_ram_chip_enabled(&self) -> bool {
        !self.mapper_state_ref().sram.is_empty() && (self.bank_select & 0x20) != 0
    }

    fn mmc6_ram_address(index: usize) -> Option<(usize, bool)> {
        if !(0x1000..=0x1FFF).contains(&index) {
            return None;
        }

        let address = (index - 0x1000) & 0x03FF;
        Some((address, address >= 0x0200))
    }

    fn mmc6_bank_read_enabled(&self, high_bank: bool) -> bool {
        if high_bank {
            (self.program_ram_protect & 0x80) != 0
        } else {
            (self.program_ram_protect & 0x20) != 0
        }
    }

    fn mmc6_bank_write_enabled(&self, high_bank: bool) -> bool {
        if high_bank {
            (self.program_ram_protect & 0x40) != 0
        } else {
            (self.program_ram_protect & 0x10) != 0
        }
    }

    fn write_bank_select(&mut self, value: u8) {
        self.bank_select = value;
        if self.is_mmc6() && !self.mmc6_program_ram_chip_enabled() {
            self.program_ram_protect = 0;
        }
        self.update_offsets();
    }

    fn write_bank_data(&mut self, value: u8) {
        let selecter = (self.bank_select & 0x07) as usize;
        // For banks 0 and 1, low bit is ignored
        self.bank_data[selecter] = if selecter <= 1 { value & !0x01 } else { value };
        self.update_offsets();
    }

    fn write_mirroring(&mut self, value: u8) {
        self.mirroring = value;

        if !matches!(self.get_mirror_mode(), MirrorMode::Four) {
            self.set_mirror_mode(match value & 1 {
                0 => MirrorMode::Vertical,
                1 => MirrorMode::Horizontal,
                _ => unreachable!(),
            });
        }
    }

    fn write_irq_latch(&mut self, value: u8) {
        self.irq_latch = value;
    }

    fn write_irq_reload(&mut self, _value: u8) {
        self.irq_counter = 0;
        self.irq_reload = true;
    }

    fn write_disable_irq(&mut self, _value: u8, interrupt: &mut Interrupt) {
        self.irq_enabled = false;
        interrupt.clear_irq(IrqSource::EXTERNAL);
    }

    fn write_enable_irq(&mut self, _value: u8) {
        self.irq_enabled = true;
    }

    fn write_program_ram_protect(&mut self, value: u8) {
        if self.is_mmc6() {
            if self.mmc6_program_ram_chip_enabled() {
                self.program_ram_protect = value & 0xF0;
            }
        } else {
            self.program_ram_protect = value;
        }
        self.update_offsets();
    }

    fn uses_old_style_irq(&self) -> bool {
        if self.is_mmc6() {
            return false;
        }

        match self.data_ref().mmc3_irq_variant_override() {
            Some(Mmc3IrqVariant::Sharp) => false,
            Some(Mmc3IrqVariant::Nec) => true,
            None => self.data_ref().sub_mapper_type() == 4,
        }
    }

    fn write_control(&mut self, _value: u8) {
        // MMC3 does not use the MMC1-style control; keep for compatibility with initialize call.
        self.update_offsets();
    }

    fn update_offsets(&mut self) {
        let prg_bank_count = self.program_bank_count();
        let last_bank = prg_bank_count.saturating_sub(1);
        let second_last_bank = prg_bank_count.saturating_sub(2);

        if (self.bank_select & 0x40) == 0 {
            self.map_program_bank(0, usize::from(self.bank_data[6]));
            self.map_program_bank(1, usize::from(self.bank_data[7]));
            self.map_program_bank(2, second_last_bank);
            self.map_program_bank(3, last_bank);
        } else {
            self.map_program_bank(0, second_last_bank);
            self.map_program_bank(1, usize::from(self.bank_data[7]));
            self.map_program_bank(2, usize::from(self.bank_data[6]));
            self.map_program_bank(3, last_bank);
        }

        if (self.bank_select & 0x80) == 0 {
            self.map_character_bank(0, usize::from(self.bank_data[0] & !0x01));
            self.map_character_bank(1, usize::from(self.bank_data[0] | 0x01));
            self.map_character_bank(2, usize::from(self.bank_data[1] & !0x01));
            self.map_character_bank(3, usize::from(self.bank_data[1] | 0x01));
            self.map_character_bank(4, usize::from(self.bank_data[2]));
            self.map_character_bank(5, usize::from(self.bank_data[3]));
            self.map_character_bank(6, usize::from(self.bank_data[4]));
            self.map_character_bank(7, usize::from(self.bank_data[5]));
        } else {
            self.map_character_bank(0, usize::from(self.bank_data[2]));
            self.map_character_bank(1, usize::from(self.bank_data[3]));
            self.map_character_bank(2, usize::from(self.bank_data[4]));
            self.map_character_bank(3, usize::from(self.bank_data[5]));
            self.map_character_bank(4, usize::from(self.bank_data[0] & !0x01));
            self.map_character_bank(5, usize::from(self.bank_data[0] | 0x01));
            self.map_character_bank(6, usize::from(self.bank_data[1] & !0x01));
            self.map_character_bank(7, usize::from(self.bank_data[1] | 0x01));
        }

        if self.is_mmc6() {
            self.clear_ram_mapping();
        } else if self.program_ram_enabled() {
            self.change_ram_page(0, 0);
        } else {
            self.clear_ram_mapping();
        }
    }

    fn clock_irq_counter(&mut self, interrupt: &mut Interrupt) {
        let counter = self.irq_counter;
        let reload_pending = self.irq_reload;

        if counter == 0 || reload_pending {
            self.irq_counter = self.irq_latch;
            self.irq_reload = false;
        } else {
            self.irq_counter = counter.wrapping_sub(1);
        }

        let irq_triggered = if self.uses_old_style_irq() {
            (!reload_pending && counter == 1) || (reload_pending && self.irq_counter == 0)
        } else {
            self.irq_counter == 0
        };

        if irq_triggered && self.irq_enabled {
            interrupt.set_irq(IrqSource::EXTERNAL);
        }
    }
}

impl CartridgeDataDao for Mmc3 {
    fn data_mut(&mut self) -> &mut CartridgeData {
        &mut self.cartridge_data
    }
    fn data_ref(&self) -> &CartridgeData {
        &self.cartridge_data
    }
}

impl MapperStateDao for Mmc3 {
    fn mapper_state_mut(&mut self) -> &mut MapperState {
        &mut self.state
    }
    fn mapper_state_ref(&self) -> &MapperState {
        &self.state
    }
}

impl Mapper for Mmc3 {
    fn program_page_len(&self) -> usize {
        0x2000
    }

    fn character_page_len(&self) -> usize {
        0x0400
    }

    fn read_ram(&self, index: usize) -> Option<u8> {
        if self.is_mmc6() {
            let (address, high_bank) = Self::mmc6_ram_address(index)?;
            if !self.mmc6_program_ram_chip_enabled() {
                return None;
            }

            let low_bank_read_enabled = self.mmc6_bank_read_enabled(false);
            let high_bank_read_enabled = self.mmc6_bank_read_enabled(true);
            if !low_bank_read_enabled && !high_bank_read_enabled {
                return None;
            }

            if self.mmc6_bank_read_enabled(high_bank) {
                Some(self.mapper_state_ref().sram[address])
            } else {
                Some(0)
            }
        } else if self.program_ram_enabled() {
            self.ram_address(index)
                .map(|address| self.mapper_state_ref().sram[address])
        } else {
            None
        }
    }

    fn write_ram(&mut self, index: usize, data: u8) {
        if self.is_mmc6() {
            let Some((address, high_bank)) = Self::mmc6_ram_address(index) else {
                return;
            };
            if self.mmc6_program_ram_chip_enabled()
                && self.mmc6_bank_read_enabled(high_bank)
                && self.mmc6_bank_write_enabled(high_bank)
            {
                self.mapper_state_mut().sram[address] = data;
            }
        } else if self.program_ram_write_enabled()
            && let Some(address) = self.ram_address(index)
        {
            self.mapper_state_mut().sram[address] = data;
        }
    }

    fn save_len_default(&self) -> usize {
        if self.data_ref().sub_mapper_type() == 1 {
            0x0400
        } else {
            0x2000
        }
    }

    fn ram_len_default(&self) -> usize {
        if self.data_ref().sub_mapper_type() == 1 {
            0x0400
        } else {
            0x2000
        }
    }

    fn ram_page_len_default(&self) -> usize {
        if self.data_ref().sub_mapper_type() == 1 {
            0x0200
        } else {
            0x2000
        }
    }

    fn battery_default(&self) -> bool {
        true
    }
    fn initialize(&mut self) {
        self.program_ram_protect = if self.is_mmc6() { 0 } else { 0x80 };
        self.write_control(0);
    }

    fn name(&self) -> &str {
        "MMC3 (Mapper4)"
    }

    fn bus_conflicts(&self) -> bool {
        self.data_ref().sub_mapper_type() == 2
    }

    fn write_register(&mut self, address: usize, value: u8, interrupt: &mut Interrupt) {
        match address & 0x6001 {
            0x0000 => self.write_bank_select(value),
            0x0001 => self.write_bank_data(value),
            0x2000 => self.write_mirroring(value),
            0x2001 => self.write_program_ram_protect(value),
            0x4000 => self.write_irq_latch(value),
            0x4001 => self.write_irq_reload(value),
            0x6000 => self.write_disable_irq(value, interrupt),
            0x6001 => self.write_enable_irq(value),
            _ => {}
        }
    }

    fn vram_address_change(
        &mut self,
        address: usize,
        ppu_tick: u64,
        _address_register_change: bool,
        interrupt: &mut Interrupt,
    ) {
        let a12_high = (address & 0x1000) != 0;

        if a12_high {
            if !self.last_a12_high
                && ppu_tick.saturating_sub(self.last_a12_low_tick) >= A12_LOW_FILTER_TICKS
            {
                self.clock_irq_counter(interrupt);
            }
        } else if self.last_a12_high {
            self.last_a12_low_tick = ppu_tick;
        }

        self.last_a12_high = a12_high;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cartridge::Cartridge;
    use crate::cartridge::format::CartridgeData;
    use crate::cpu::interrupt::{Interrupt, IrqSource};

    fn new_mapper(sub_mapper_type: u8) -> Mmc3 {
        let mut rom = vec![
            0x4E,
            0x45,
            0x53,
            0x1A,
            0x02,
            0x01,
            0x40,
            0x08,
            sub_mapper_type << 4,
            0x00,
            0x00,
            0x00,
            0x00,
            0x00,
            0x00,
            0x00,
        ];
        rom.resize(16 + 0x8000 + 0x2000, 0);
        let data =
            CartridgeData::try_from(&mut rom.into_iter()).expect("cartridge data should parse");
        let mut mapper = Mmc3::new(data);
        Cartridge::initialize(&mut mapper);
        mapper
    }

    #[test]
    fn mmc6_maps_ram_at_7000_with_1kb_mirroring() {
        let mut mapper = new_mapper(1);

        mapper.write_bank_select(0x20);
        mapper.write_program_ram_protect(0xF0);
        Mapper::write_ram(&mut mapper, 0x1000, 0x12);
        Mapper::write_ram(&mut mapper, 0x1200, 0x34);

        assert_eq!(Mapper::read_ram(&mapper, 0x0000), None);
        assert_eq!(Mapper::read_ram(&mapper, 0x1000), Some(0x12));
        assert_eq!(Mapper::read_ram(&mapper, 0x1400), Some(0x12));
        assert_eq!(Mapper::read_ram(&mapper, 0x1200), Some(0x34));
        assert_eq!(Mapper::read_ram(&mapper, 0x1600), Some(0x34));
    }

    #[test]
    fn mmc6_respects_chip_enable_and_half_bank_permissions() {
        let mut mapper = new_mapper(1);

        mapper.write_program_ram_protect(0xF0);
        assert_eq!(Mapper::read_ram(&mapper, 0x1000), None);

        mapper.write_bank_select(0x20);
        assert_eq!(Mapper::read_ram(&mapper, 0x1000), None);

        mapper.write_program_ram_protect(0x30);
        Mapper::write_ram(&mut mapper, 0x1000, 0x56);
        Mapper::write_ram(&mut mapper, 0x1200, 0x78);

        assert_eq!(Mapper::read_ram(&mapper, 0x1000), Some(0x56));
        assert_eq!(Mapper::read_ram(&mapper, 0x1200), Some(0x00));

        mapper.write_bank_select(0x00);
        assert_eq!(Mapper::read_ram(&mapper, 0x1000), None);

        mapper.write_bank_select(0x20);
        assert_eq!(Mapper::read_ram(&mapper, 0x1000), None);
    }

    #[test]
    fn mmc6_defaults_to_sharp_irq_behavior() {
        let mapper = new_mapper(1);

        assert!(!mapper.uses_old_style_irq());
    }

    #[test]
    fn mmc6_cpu_6000_reads_as_open_bus_zero() {
        let mapper = new_mapper(1);
        let read_result = Cartridge::read(&mapper, 0x6000);

        assert_eq!(read_result.data, 0);
        assert_eq!(read_result.mask, 0);
    }

    #[test]
    fn register_changes_still_require_filtered_a12_low_time() {
        let mut mapper = new_mapper(0);
        let mut interrupt = Interrupt::new();
        mapper.irq_counter = 1;
        mapper.irq_enabled = true;

        mapper.vram_address_change(0x0FFF, 0, true, &mut interrupt);
        mapper.vram_address_change(0x1000, 8, true, &mut interrupt);

        assert_eq!(mapper.irq_counter, 1);
        assert!(!interrupt.get_irq(IrqSource::EXTERNAL));

        mapper.vram_address_change(0x0FFF, 9, true, &mut interrupt);
        mapper.vram_address_change(0x1000, 18, true, &mut interrupt);

        assert_eq!(mapper.irq_counter, 0);
        assert!(interrupt.get_irq(IrqSource::EXTERNAL));
    }
}
