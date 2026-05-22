// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use crate::cpu::interrupt::Interrupt;
use crate::mapper_state::{MapperStateDao, MappingMode};
use crate::ppu_bus_event::PpuBusEvent;
use crate::{CartridgeData, MirrorMode, OpenBusReadResult};

pub(crate) trait CartridgeDataDao {
    fn data_mut(&mut self) -> &mut CartridgeData;
    fn data_ref(&self) -> &CartridgeData;
}

pub(crate) trait Mapper: MapperStateDao + CartridgeDataDao {
    fn name(&self) -> &str;
    fn program_page_len(&self) -> usize;
    fn character_page_len(&self) -> usize;
    fn initialize(&mut self);
    fn ram_page_len(&self) -> usize {
        0x2000
    }

    fn register_addr(&self, address: usize) -> bool {
        address >= 0x8000
    }

    fn write_register(&mut self, _address: usize, _value: u8, _interrupt: &mut Interrupt) {}

    /// Schedule a mapper register write for the current CPU cycle.
    ///
    /// Most mappers apply the write immediately. Mappers with CPU/PPU phase-sensitive behavior may
    /// defer the write until `flush_deferred_register_writes` is called.
    fn schedule_register_write(&mut self, address: usize, value: u8, interrupt: &mut Interrupt) {
        self.write_register(address, value, interrupt);
    }

    /// Apply any writes previously deferred by `schedule_register_write`.
    fn flush_deferred_register_writes(&mut self, _interrupt: &mut Interrupt) {}

    fn read_expansion(&self, _address: usize) -> OpenBusReadResult {
        OpenBusReadResult::new(0, 0)
    }

    fn write_expansion(&mut self, _address: usize, _value: u8, _interrupt: &mut Interrupt) {}

    fn battery_default(&self) -> bool {
        false
    }

    fn save_len_default(&self) -> usize {
        if self.battery_default() { 0x2000 } else { 0 }
    }

    fn ram_len_default(&self) -> usize {
        if self.battery_default() { 0x2000 } else { 0 }
    }

    fn ram_page_len_default(&self) -> usize {
        if self.battery_default() { 0x2000 } else { 0 }
    }

    fn character_ram_page_len_default(&self) -> usize {
        0x2000
    }

    // fn character_ram_page_len(&self) -> usize;

    fn bus_conflicts(&self) -> bool {
        false
    }

    fn change_program_page(&mut self, offset: usize, page: usize) {
        let total_pages = self.data_ref().prog_rom_len() >> 8;
        let page_count = self.program_page_len() >> 8;
        let page_offset = offset * page_count;
        let mut page_value_offset = page * page_count;
        for i in page_offset..(page_offset + page_count) {
            while page_value_offset >= total_pages {
                page_value_offset -= total_pages;
            }
            self.mapper_state_mut().program_page_table[i] = Some(page_value_offset);
            page_value_offset += 1;
        }
    }

    fn change_character_page(&mut self, offset: usize, page: usize) {
        let total_pages = if self.mapper_state_ref().character_mapping_mode == MappingMode::Ram {
            self.mapper_state_ref().vram.len()
        } else {
            self.data_ref().char_rom_len()
        } >> 8;
        let page_count = self.character_page_len() >> 8;
        let page_offset = offset * page_count;
        let mut page_value_offset = page * page_count;
        for i in page_offset..(page_offset + page_count) {
            while page_value_offset >= total_pages {
                page_value_offset -= total_pages;
            }
            self.mapper_state_mut().character_page_table[i] = Some(page_value_offset);
            page_value_offset += 1;
        }
    }

    fn release_character_page(&mut self, offset: usize) {
        let page_count = self.character_page_len() >> 8;
        let page_offset = offset * page_count;
        for i in page_offset..(page_offset + page_count) {
            self.mapper_state_mut().character_page_table[i] = None;
        }
    }

    fn change_ram_page(&mut self, offset: usize, page: usize) {
        let total_pages = self.mapper_state_mut().sram.len() >> 8;
        if total_pages > 0 {
            let page_count = self.ram_page_len() >> 8;
            let page_offset = offset * page_count;
            let mut page_value_offset = page * page_count;
            for i in page_offset..(page_offset + page_count) {
                while page_value_offset >= total_pages {
                    page_value_offset -= total_pages;
                }
                self.mapper_state_mut().sram_page_table[i] = Some(page_value_offset);
                page_value_offset += 1;
            }
        }
    }

    fn program_address(&self, address: usize) -> Option<usize> {
        self.mapper_state_ref().program_page_table[address >> 8]
            .map(|x| (x << 8) | (address & 0xFF))
    }

    fn character_address(&self, address: usize) -> Option<usize> {
        self.mapper_state_ref().character_page_table[address >> 8]
            .map(|x| (x << 8) | (address & 0xFF))
    }

    fn ram_address(&self, address: usize) -> Option<usize> {
        self.mapper_state_ref().sram_page_table[address >> 8].map(|x| (x << 8) | (address & 0xFF))
    }

    fn character_openbus_default(&self) -> Option<u8> {
        None
    }

    fn get_mirror_mode(&self) -> MirrorMode {
        self.mapper_state_ref().mirror_mode
    }

    fn set_mirror_mode(&mut self, value: MirrorMode) {
        self.mapper_state_mut().mirror_mode = value;
    }

    fn read_ram(&self, index: usize) -> Option<u8> {
        self.ram_address(index)
            .map(|x| self.mapper_state_ref().sram[x])
    }

    fn write_ram(&mut self, index: usize, data: u8) {
        if let Some(addr) = self.ram_address(index) {
            self.mapper_state_mut().sram[addr] = data;
        }
    }

    fn step(&mut self, _interrupt: &mut Interrupt) {}

    /// Notify the mapper of an observable PPU bus event.
    ///
    /// The default implementation is a no-op. Mappers that count scanlines or
    /// otherwise react to PPU bus activity (e.g. MMC3-family A12-edge IRQ) override
    /// this method.
    fn notify_ppu_bus_event(&mut self, _event: PpuBusEvent, _interrupt: &mut Interrupt) {}
}
