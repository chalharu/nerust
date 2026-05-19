// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use crate::MirrorMode;
use crate::cartridge_api::{CartridgeDataDao, Mapper, MapperState, MapperStateDao, PpuBusEvent};
use crate::cartridge_data::CartridgeData;
use crate::cpu::interrupt::{Interrupt, IrqSource};

const A12_LOW_FILTER_TICKS: u64 = 9;

#[derive(serde_derive::Serialize, serde_derive::Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum IrqVariant {
    Sharp,
    NecOldStyle,
}

#[derive(serde_derive::Serialize, serde_derive::Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum PrgRamModel {
    Standard,
    Mmc6,
}

#[derive(
    serde_derive::Serialize, serde_derive::Deserialize, Clone, Copy, PartialEq, Eq, Debug, Default,
)]
pub(super) enum MirroringModel {
    #[default]
    Standard,
    TxSrom,
}

#[derive(serde_derive::Serialize, serde_derive::Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
pub(super) struct Mapper4Config {
    irq_variant: IrqVariant,
    prg_ram_model: PrgRamModel,
    bus_conflicts: bool,
    #[serde(default)]
    mirroring_model: MirroringModel,
}

impl Mapper4Config {
    pub(super) const fn from_parts(
        irq_variant: IrqVariant,
        prg_ram_model: PrgRamModel,
        bus_conflicts: bool,
    ) -> Self {
        Self::from_parts_with_mirroring(
            irq_variant,
            prg_ram_model,
            bus_conflicts,
            MirroringModel::Standard,
        )
    }

    pub(super) const fn from_parts_with_mirroring(
        irq_variant: IrqVariant,
        prg_ram_model: PrgRamModel,
        bus_conflicts: bool,
        mirroring_model: MirroringModel,
    ) -> Self {
        Self {
            irq_variant,
            prg_ram_model,
            bus_conflicts,
            mirroring_model,
        }
    }

    pub(super) const fn mmc3(bus_conflicts: bool) -> Self {
        Self::from_parts(IrqVariant::Sharp, PrgRamModel::Standard, bus_conflicts)
    }

    pub(super) const fn mmc3_nec(bus_conflicts: bool) -> Self {
        Self::from_parts(
            IrqVariant::NecOldStyle,
            PrgRamModel::Standard,
            bus_conflicts,
        )
    }

    pub(super) const fn mmc6() -> Self {
        Self::from_parts(IrqVariant::NecOldStyle, PrgRamModel::Mmc6, false)
    }

    pub(super) const fn txsrom() -> Self {
        Self::from_parts_with_mirroring(
            IrqVariant::Sharp,
            PrgRamModel::Standard,
            false,
            MirroringModel::TxSrom,
        )
    }
}

#[derive(serde_derive::Serialize, serde_derive::Deserialize)]
struct IrqUnit {
    variant: IrqVariant,
    latch: u8,
    reload: bool,
    counter: u8,
    enabled: bool,
    last_a12_high: bool,
    last_a12_low_tick: u64,
}

impl IrqUnit {
    fn new(variant: IrqVariant) -> Self {
        Self {
            variant,
            latch: 0,
            reload: false,
            counter: 0,
            enabled: false,
            last_a12_high: false,
            last_a12_low_tick: 0,
        }
    }

    fn write_latch(&mut self, value: u8) {
        self.latch = value;
    }

    fn write_reload(&mut self) {
        self.counter = 0;
        self.reload = true;
    }

    fn write_disable(&mut self, interrupt: &mut Interrupt) {
        self.enabled = false;
        interrupt.clear_irq(IrqSource::EXTERNAL);
    }

    fn write_enable(&mut self) {
        self.enabled = true;
    }

    fn clock(&mut self, interrupt: &mut Interrupt) {
        let counter = self.counter;
        let reload_pending = self.reload;

        if counter == 0 || reload_pending {
            self.counter = self.latch;
            self.reload = false;
        } else {
            self.counter = counter.wrapping_sub(1);
        }

        let irq_triggered = match self.variant {
            IrqVariant::NecOldStyle => {
                (!reload_pending && counter == 1) || (reload_pending && self.counter == 0)
            }
            IrqVariant::Sharp => self.counter == 0,
        };

        if irq_triggered && self.enabled {
            interrupt.set_irq(IrqSource::EXTERNAL);
        }
    }

    fn on_address_bus_update(&mut self, address: usize, ppu_tick: u64, interrupt: &mut Interrupt) {
        let a12_high = (address & 0x1000) != 0;

        if a12_high {
            if !self.last_a12_high
                && ppu_tick.saturating_sub(self.last_a12_low_tick) >= A12_LOW_FILTER_TICKS
            {
                self.clock(interrupt);
            }
        } else if self.last_a12_high {
            self.last_a12_low_tick = ppu_tick;
        }

        self.last_a12_high = a12_high;
    }
}

pub(super) struct LegacyIrqState {
    pub(super) variant: IrqVariant,
    pub(super) latch: u8,
    pub(super) reload: bool,
    pub(super) counter: u8,
    pub(super) enabled: bool,
    pub(super) last_a12_high: bool,
    pub(super) last_a12_low_tick: u64,
}

pub(super) struct LegacyMapper4State {
    pub(super) state: MapperState,
    pub(super) bank_select: u8,
    pub(super) bank_data: [u8; 8],
    pub(super) mirroring: u8,
    pub(super) program_ram_protect: u8,
    pub(super) irq: LegacyIrqState,
    pub(super) prg_ram_model: PrgRamModel,
}

#[derive(serde_derive::Serialize, serde_derive::Deserialize)]
pub(super) struct Mapper4Shared {
    cartridge_data: CartridgeData,
    state: MapperState,
    bank_select: u8,
    bank_data: [u8; 8],
    mirroring: u8,
    program_ram_protect: u8,
    irq: IrqUnit,
    config: Mapper4Config,
}

impl Mapper4Shared {
    pub(super) fn new(data: CartridgeData, config: Mapper4Config) -> Self {
        Self {
            cartridge_data: data,
            state: MapperState::new(),
            bank_select: 0,
            bank_data: [0, 0, 0, 0, 0, 0, 0, 1],
            mirroring: 0,
            program_ram_protect: 0x80,
            irq: IrqUnit::new(config.irq_variant),
            config,
        }
    }

    pub(super) fn from_legacy_state(
        cartridge_data: CartridgeData,
        legacy: LegacyMapper4State,
    ) -> Self {
        Self {
            config: Mapper4Config::from_parts(
                legacy.irq.variant,
                legacy.prg_ram_model,
                cartridge_data.sub_mapper_type() == 2,
            ),
            irq: IrqUnit {
                variant: legacy.irq.variant,
                latch: legacy.irq.latch,
                reload: legacy.irq.reload,
                counter: legacy.irq.counter,
                enabled: legacy.irq.enabled,
                last_a12_high: legacy.irq.last_a12_high,
                last_a12_low_tick: legacy.irq.last_a12_low_tick,
            },
            cartridge_data,
            state: legacy.state,
            bank_select: legacy.bank_select,
            bank_data: legacy.bank_data,
            mirroring: legacy.mirroring,
            program_ram_protect: legacy.program_ram_protect,
        }
    }

    fn clear_ram_mapping(&mut self) {
        for slot in &mut self.mapper_state_mut().sram_page_table {
            *slot = None;
        }
    }

    fn program_bank_count(&self) -> usize {
        self.data_ref().prog_rom_len() / 0x2000
    }

    fn character_bank_count(&self) -> usize {
        if self.mapper_state_ref().character_mapping_mode == crate::cartridge_api::MappingMode::Ram
        {
            self.mapper_state_ref().vram.len() / 0x0400
        } else {
            self.data_ref().char_rom_len() / 0x0400
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
        match self.config.prg_ram_model {
            PrgRamModel::Mmc6 => self.mmc6_chip_enabled(),
            PrgRamModel::Standard => {
                !self.mapper_state_ref().sram.is_empty() && (self.program_ram_protect & 0x80) != 0
            }
        }
    }

    fn program_ram_write_enabled(&self) -> bool {
        match self.config.prg_ram_model {
            PrgRamModel::Mmc6 => self.mmc6_chip_enabled(),
            PrgRamModel::Standard => {
                self.program_ram_enabled() && (self.program_ram_protect & 0x40) == 0
            }
        }
    }

    fn mmc6_chip_enabled(&self) -> bool {
        !self.mapper_state_ref().sram.is_empty() && (self.bank_select & 0x20) != 0
    }

    fn mmc6_ram_address(index: usize) -> Option<(usize, bool)> {
        if !(0x1000..=0x1FFF).contains(&index) {
            return None;
        }
        let address = (index - 0x1000) & 0x03FF;
        Some((address, address >= 0x0200))
    }

    fn mmc6_half_bank_read_enabled(&self, high_bank: bool) -> bool {
        if high_bank {
            (self.program_ram_protect & 0x80) != 0
        } else {
            (self.program_ram_protect & 0x20) != 0
        }
    }

    fn mmc6_half_bank_write_enabled(&self, high_bank: bool) -> bool {
        if high_bank {
            (self.program_ram_protect & 0x40) != 0
        } else {
            (self.program_ram_protect & 0x10) != 0
        }
    }

    fn write_bank_select(&mut self, value: u8) {
        self.bank_select = value;
        if self.config.prg_ram_model == PrgRamModel::Mmc6 && !self.mmc6_chip_enabled() {
            self.program_ram_protect = 0;
        }
        self.update_offsets();
    }

    fn write_bank_data(&mut self, value: u8) {
        let selecter = (self.bank_select & 0x07) as usize;
        self.bank_data[selecter] = if selecter <= 1 { value & !0x01 } else { value };
        self.update_offsets();
    }

    fn write_mirroring(&mut self, value: u8) {
        self.mirroring = value;
        self.update_mirroring_mode();
    }

    fn update_mirroring_mode(&mut self) {
        match self.config.mirroring_model {
            MirroringModel::Standard => {
                if !matches!(self.get_mirror_mode(), MirrorMode::Four) {
                    self.set_mirror_mode(match self.mirroring & 1 {
                        0 => MirrorMode::Vertical,
                        1 => MirrorMode::Horizontal,
                        _ => unreachable!(),
                    });
                }
            }
            MirroringModel::TxSrom => {
                let nametable_registers = if (self.bank_select & 0x80) == 0 {
                    [0, 0, 1, 1]
                } else {
                    [2, 3, 4, 5]
                };
                self.set_mirror_mode(MirrorMode::Custom(
                    nametable_registers.map(|register| (self.bank_data[register] >> 7) & 0x01),
                ));
            }
        }
    }

    fn write_program_ram_protect(&mut self, value: u8) {
        match self.config.prg_ram_model {
            PrgRamModel::Mmc6 => {
                if self.mmc6_chip_enabled() {
                    self.program_ram_protect = value & 0xF0;
                }
            }
            PrgRamModel::Standard => {
                self.program_ram_protect = value;
            }
        }
        self.update_offsets();
    }

    fn write_control(&mut self, _value: u8) {
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

        match self.config.prg_ram_model {
            PrgRamModel::Mmc6 => self.clear_ram_mapping(),
            PrgRamModel::Standard => {
                if self.program_ram_enabled() {
                    self.change_ram_page(0, 0);
                } else {
                    self.clear_ram_mapping();
                }
            }
        }

        self.update_mirroring_mode();
    }

    #[cfg(test)]
    pub(super) fn irq_variant(&self) -> IrqVariant {
        self.irq.variant
    }

    #[cfg(test)]
    pub(super) fn prg_ram_model(&self) -> PrgRamModel {
        self.config.prg_ram_model
    }

    #[cfg(test)]
    pub(super) fn irq_counter(&self) -> u8 {
        self.irq.counter
    }

    #[cfg(test)]
    pub(super) fn set_irq_counter(&mut self, value: u8) {
        self.irq.counter = value;
    }

    #[cfg(test)]
    pub(super) fn set_irq_enabled(&mut self, value: bool) {
        self.irq.enabled = value;
    }

    #[cfg(test)]
    pub(super) fn write_irq_latch(&mut self, value: u8) {
        self.irq.write_latch(value);
    }

    #[cfg(test)]
    pub(super) fn write_irq_reload(&mut self) {
        self.irq.write_reload();
    }

    #[cfg(test)]
    pub(super) fn clock_irq(&mut self, interrupt: &mut Interrupt) {
        self.irq.clock(interrupt);
    }
}

impl CartridgeDataDao for Mapper4Shared {
    fn data_mut(&mut self) -> &mut CartridgeData {
        &mut self.cartridge_data
    }

    fn data_ref(&self) -> &CartridgeData {
        &self.cartridge_data
    }
}

impl MapperStateDao for Mapper4Shared {
    fn mapper_state_mut(&mut self) -> &mut MapperState {
        &mut self.state
    }

    fn mapper_state_ref(&self) -> &MapperState {
        &self.state
    }
}

impl Mapper for Mapper4Shared {
    fn name(&self) -> &str {
        "Mapper4 family shared core"
    }

    fn program_page_len(&self) -> usize {
        0x2000
    }

    fn character_page_len(&self) -> usize {
        0x0400
    }

    fn read_ram(&self, index: usize) -> Option<u8> {
        match self.config.prg_ram_model {
            PrgRamModel::Mmc6 => {
                let (address, high_bank) = Self::mmc6_ram_address(index)?;
                if !self.mmc6_chip_enabled() {
                    return None;
                }
                let low_read = self.mmc6_half_bank_read_enabled(false);
                let high_read = self.mmc6_half_bank_read_enabled(true);
                if !low_read && !high_read {
                    return None;
                }
                if self.mmc6_half_bank_read_enabled(high_bank) {
                    Some(self.mapper_state_ref().sram[address])
                } else {
                    Some(0)
                }
            }
            PrgRamModel::Standard => {
                if self.program_ram_enabled() {
                    self.ram_address(index)
                        .map(|address| self.mapper_state_ref().sram[address])
                } else {
                    None
                }
            }
        }
    }

    fn write_ram(&mut self, index: usize, data: u8) {
        match self.config.prg_ram_model {
            PrgRamModel::Mmc6 => {
                let Some((address, high_bank)) = Self::mmc6_ram_address(index) else {
                    return;
                };
                if self.mmc6_chip_enabled()
                    && self.mmc6_half_bank_read_enabled(high_bank)
                    && self.mmc6_half_bank_write_enabled(high_bank)
                {
                    self.mapper_state_mut().sram[address] = data;
                }
            }
            PrgRamModel::Standard => {
                if self.program_ram_write_enabled()
                    && let Some(address) = self.ram_address(index)
                {
                    self.mapper_state_mut().sram[address] = data;
                }
            }
        }
    }

    fn save_len_default(&self) -> usize {
        match self.config.prg_ram_model {
            PrgRamModel::Mmc6 => 0x0400,
            PrgRamModel::Standard => 0x2000,
        }
    }

    fn ram_len_default(&self) -> usize {
        match self.config.prg_ram_model {
            PrgRamModel::Mmc6 => 0x0400,
            PrgRamModel::Standard => 0x2000,
        }
    }

    fn ram_page_len_default(&self) -> usize {
        match self.config.prg_ram_model {
            PrgRamModel::Mmc6 => 0x0200,
            PrgRamModel::Standard => 0x2000,
        }
    }

    fn battery_default(&self) -> bool {
        true
    }

    fn initialize(&mut self) {
        self.program_ram_protect = match self.config.prg_ram_model {
            PrgRamModel::Mmc6 => 0,
            PrgRamModel::Standard => 0x80,
        };
        self.write_control(0);
    }

    fn bus_conflicts(&self) -> bool {
        self.config.bus_conflicts
    }

    fn write_register(&mut self, address: usize, value: u8, interrupt: &mut Interrupt) {
        match address & 0x6001 {
            0x0000 => self.write_bank_select(value),
            0x0001 => self.write_bank_data(value),
            0x2000 => self.write_mirroring(value),
            0x2001 => self.write_program_ram_protect(value),
            0x4000 => self.irq.write_latch(value),
            0x4001 => self.irq.write_reload(),
            0x6000 => self.irq.write_disable(interrupt),
            0x6001 => self.irq.write_enable(),
            _ => {}
        }
    }

    fn notify_ppu_bus_event(&mut self, event: PpuBusEvent, interrupt: &mut Interrupt) {
        let PpuBusEvent::AddressBusUpdate {
            address, ppu_tick, ..
        } = event;
        self.irq.on_address_bus_update(address, ppu_tick, interrupt);
    }
}

pub(super) trait Mapper4Wrapper {
    const NAME: &'static str;

    fn shared_ref(&self) -> &Mapper4Shared;
    fn shared_mut(&mut self) -> &mut Mapper4Shared;
}

impl<T> CartridgeDataDao for T
where
    T: Mapper4Wrapper,
{
    fn data_mut(&mut self) -> &mut CartridgeData {
        self.shared_mut().data_mut()
    }

    fn data_ref(&self) -> &CartridgeData {
        self.shared_ref().data_ref()
    }
}

impl<T> MapperStateDao for T
where
    T: Mapper4Wrapper,
{
    fn mapper_state_mut(&mut self) -> &mut MapperState {
        self.shared_mut().mapper_state_mut()
    }

    fn mapper_state_ref(&self) -> &MapperState {
        self.shared_ref().mapper_state_ref()
    }
}

impl<T> Mapper for T
where
    T: Mapper4Wrapper,
{
    fn name(&self) -> &str {
        Self::NAME
    }

    fn program_page_len(&self) -> usize {
        self.shared_ref().program_page_len()
    }

    fn character_page_len(&self) -> usize {
        self.shared_ref().character_page_len()
    }

    fn read_ram(&self, index: usize) -> Option<u8> {
        self.shared_ref().read_ram(index)
    }

    fn write_ram(&mut self, index: usize, data: u8) {
        self.shared_mut().write_ram(index, data);
    }

    fn save_len_default(&self) -> usize {
        self.shared_ref().save_len_default()
    }

    fn ram_len_default(&self) -> usize {
        self.shared_ref().ram_len_default()
    }

    fn ram_page_len_default(&self) -> usize {
        self.shared_ref().ram_page_len_default()
    }

    fn battery_default(&self) -> bool {
        self.shared_ref().battery_default()
    }

    fn initialize(&mut self) {
        self.shared_mut().initialize();
    }

    fn bus_conflicts(&self) -> bool {
        self.shared_ref().bus_conflicts()
    }

    fn write_register(&mut self, address: usize, value: u8, interrupt: &mut Interrupt) {
        self.shared_mut().write_register(address, value, interrupt);
    }

    fn notify_ppu_bus_event(&mut self, event: PpuBusEvent, interrupt: &mut Interrupt) {
        self.shared_mut().notify_ppu_bus_event(event, interrupt);
    }
}
