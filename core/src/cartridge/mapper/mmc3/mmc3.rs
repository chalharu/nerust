// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use super::shared::{Mapper4Config, Mapper4Shared};
use crate::cartridge::format::CartridgeData;
use crate::cartridge::{
    Cartridge, CartridgeDataDao, Mapper, MapperState, MapperStateDao, PpuBusEvent,
};
use crate::cpu::interrupt::Interrupt;

#[derive(serde_derive::Serialize)]
pub(super) struct Mmc3 {
    pub(super) shared: Mapper4Shared,
}

#[derive(serde_derive::Deserialize)]
#[serde(untagged)]
enum Mmc3Deserialized {
    Current { shared: Mapper4Shared },
    Legacy(LegacyMmc3State),
}

#[derive(serde_derive::Deserialize)]
struct LegacyMmc3State {
    cartridge_data: CartridgeData,
    state: MapperState,
    bank_select: u8,
    bank_data: [u8; 8],
    mirroring: u8,
    program_ram_protect: u8,
    irq: LegacyIrqUnit,
    prg_ram_model: LegacyPrgRamModel,
}

#[derive(serde_derive::Deserialize)]
enum LegacyIrqVariant {
    Sharp,
    NecOldStyle,
}

#[derive(serde_derive::Deserialize)]
struct LegacyIrqUnit {
    variant: LegacyIrqVariant,
    latch: u8,
    reload: bool,
    counter: u8,
    enabled: bool,
    last_a12_high: bool,
    last_a12_low_tick: u64,
}

#[derive(serde_derive::Deserialize)]
enum LegacyPrgRamModel {
    Standard,
    Mmc6,
}

impl Mmc3 {
    pub(super) fn new(data: CartridgeData, bus_conflicts: bool) -> Self {
        Self {
            shared: Mapper4Shared::new(data, Mapper4Config::mmc3(bus_conflicts)),
        }
    }

    fn from_deserialized(deserialized: Mmc3Deserialized) -> Self {
        match deserialized {
            Mmc3Deserialized::Current { shared } => Self { shared },
            Mmc3Deserialized::Legacy(legacy) => Self {
                shared: Mapper4Shared::from_serialized_parts(
                    legacy.cartridge_data,
                    legacy.state,
                    legacy.bank_select,
                    legacy.bank_data,
                    legacy.mirroring,
                    legacy.program_ram_protect,
                    legacy.irq.variant.into(),
                    legacy.irq.latch,
                    legacy.irq.reload,
                    legacy.irq.counter,
                    legacy.irq.enabled,
                    legacy.irq.last_a12_high,
                    legacy.irq.last_a12_low_tick,
                    legacy.prg_ram_model.into(),
                ),
            },
        }
    }
}

impl From<LegacyIrqVariant> for super::shared::IrqVariant {
    fn from(value: LegacyIrqVariant) -> Self {
        match value {
            LegacyIrqVariant::Sharp => Self::Sharp,
            LegacyIrqVariant::NecOldStyle => Self::NecOldStyle,
        }
    }
}

impl From<LegacyPrgRamModel> for super::shared::PrgRamModel {
    fn from(value: LegacyPrgRamModel) -> Self {
        match value {
            LegacyPrgRamModel::Standard => Self::Standard,
            LegacyPrgRamModel::Mmc6 => Self::Mmc6,
        }
    }
}

impl<'de> serde::Deserialize<'de> for Mmc3 {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        Ok(Self::from_deserialized(
            <Mmc3Deserialized as serde::Deserialize>::deserialize(deserializer)?,
        ))
    }
}

#[typetag::serde]
impl Cartridge for Mmc3 {}

impl CartridgeDataDao for Mmc3 {
    fn data_mut(&mut self) -> &mut CartridgeData {
        self.shared.data_mut()
    }

    fn data_ref(&self) -> &CartridgeData {
        self.shared.data_ref()
    }
}

impl MapperStateDao for Mmc3 {
    fn mapper_state_mut(&mut self) -> &mut MapperState {
        self.shared.mapper_state_mut()
    }

    fn mapper_state_ref(&self) -> &MapperState {
        self.shared.mapper_state_ref()
    }
}

impl Mapper for Mmc3 {
    fn name(&self) -> &str {
        "MMC3 (Mapper4)"
    }

    fn program_page_len(&self) -> usize {
        self.shared.program_page_len()
    }

    fn character_page_len(&self) -> usize {
        self.shared.character_page_len()
    }

    fn read_ram(&self, index: usize) -> Option<u8> {
        self.shared.read_ram(index)
    }

    fn write_ram(&mut self, index: usize, data: u8) {
        self.shared.write_ram(index, data);
    }

    fn save_len_default(&self) -> usize {
        self.shared.save_len_default()
    }

    fn ram_len_default(&self) -> usize {
        self.shared.ram_len_default()
    }

    fn ram_page_len_default(&self) -> usize {
        self.shared.ram_page_len_default()
    }

    fn battery_default(&self) -> bool {
        self.shared.battery_default()
    }

    fn initialize(&mut self) {
        self.shared.initialize();
    }

    fn bus_conflicts(&self) -> bool {
        self.shared.bus_conflicts()
    }

    fn write_register(&mut self, address: usize, value: u8, interrupt: &mut Interrupt) {
        self.shared.write_register(address, value, interrupt);
    }

    fn notify_ppu_bus_event(&mut self, event: PpuBusEvent, interrupt: &mut Interrupt) {
        self.shared.notify_ppu_bus_event(event, interrupt);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cartridge::Mapper;

    fn test_data(sub_mapper_type: u8) -> CartridgeData {
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
        CartridgeData::try_from(&mut rom.into_iter()).expect("cartridge data should parse")
    }

    #[test]
    fn legacy_flat_payload_deserializes_into_shared_wrapper() {
        let mapper = Mmc3::from_deserialized(Mmc3Deserialized::Legacy(LegacyMmc3State {
            cartridge_data: test_data(1),
            state: MapperState::new(),
            bank_select: 0x20,
            bank_data: [0, 0, 0, 0, 0, 0, 0, 1],
            mirroring: 0,
            program_ram_protect: 0xF0,
            irq: LegacyIrqUnit {
                variant: LegacyIrqVariant::NecOldStyle,
                latch: 3,
                reload: true,
                counter: 2,
                enabled: true,
                last_a12_high: false,
                last_a12_low_tick: 9,
            },
            prg_ram_model: LegacyPrgRamModel::Mmc6,
        }));

        assert_eq!(
            mapper.shared.prg_ram_model(),
            super::super::shared::PrgRamModel::Mmc6
        );
        assert_eq!(
            mapper.shared.irq_variant(),
            super::super::shared::IrqVariant::NecOldStyle
        );
        assert!(!Mapper::bus_conflicts(&mapper.shared));
    }
}
