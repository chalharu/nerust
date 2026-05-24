// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

mod mmc3_nec;
mod mmc6;
mod shared;
mod standard;
mod txsrom;

use self::mmc3_nec::Mmc3Nec;
use self::mmc6::Mmc6;
use self::standard::Mmc3;
use self::txsrom::TxSrom;
use crate::cart_device::Cartridge;
use crate::cartridge_data::CartridgeData;
use crate::cartridge_error::CartridgeError;
use nerust_contract::Mmc3IrqVariant;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mapper4Model {
    Mmc3 { bus_conflicts: bool },
    Mmc3Nec { bus_conflicts: bool },
    Mmc6,
}

fn resolve_mapper4_model(
    data: &CartridgeData,
    mmc3_irq_variant: Option<Mmc3IrqVariant>,
) -> Mapper4Model {
    if data.sub_mapper_type() == 1 {
        return Mapper4Model::Mmc6;
    }

    let bus_conflicts = data.sub_mapper_type() == 2;
    match mmc3_irq_variant {
        Some(Mmc3IrqVariant::Sharp) => Mapper4Model::Mmc3 { bus_conflicts },
        Some(Mmc3IrqVariant::Nec) => Mapper4Model::Mmc3Nec { bus_conflicts },
        None if data.sub_mapper_type() == 4 => Mapper4Model::Mmc3Nec {
            bus_conflicts: false,
        },
        None => Mapper4Model::Mmc3 { bus_conflicts },
    }
}

pub(crate) fn try_from(
    data: CartridgeData,
    mmc3_irq_variant: Option<Mmc3IrqVariant>,
) -> Result<Box<dyn Cartridge>, CartridgeError> {
    Ok(match resolve_mapper4_model(&data, mmc3_irq_variant) {
        Mapper4Model::Mmc3 { bus_conflicts } => Box::new(Mmc3::new(data, bus_conflicts)),
        Mapper4Model::Mmc3Nec { bus_conflicts } => Box::new(Mmc3Nec::new(data, bus_conflicts)),
        Mapper4Model::Mmc6 => Box::new(Mmc6::new(data)),
    })
}

pub(crate) fn try_from_txsrom(data: CartridgeData) -> Result<Box<dyn Cartridge>, CartridgeError> {
    Ok(Box::new(TxSrom::new(data)))
}

#[cfg(test)]
mod tests {
    use super::shared::{IrqVariant, PrgRamModel};
    use super::{
        Cartridge, CartridgeData, Mapper4Model, Mmc3, Mmc3IrqVariant, Mmc3Nec, Mmc6,
        resolve_mapper4_model,
    };
    use crate::cartridge_data::CartridgeDataParts;
    use crate::interrupt::{Interrupt, IrqSource};
    use crate::mapper::Mapper;
    use crate::ppu_memory_access::{PpuBusAccess, PpuBusEvent};
    use nerust_contract::MirrorMode;
    use nerust_contract::RomFormat;

    fn test_data(sub_mapper_type: u8) -> CartridgeData {
        CartridgeData::new(CartridgeDataParts {
            format: RomFormat::Nes20,
            prog_rom: vec![0; 0x8000],
            char_rom: vec![0; 0x2000],
            pram_length: 0,
            save_pram_length: 0,
            vram_length: 0,
            save_vram_length: 0,
            mapper_type: 4,
            mirror_mode: MirrorMode::Horizontal,
            has_battery: false,
            sub_mapper_type,
            trainer: Vec::new(),
        })
        .expect("test cartridge data should be valid")
    }

    fn new_mmc3(sub_mapper_type: u8) -> Mmc3 {
        let mut mapper = Mmc3::new(test_data(sub_mapper_type), sub_mapper_type == 2);
        Cartridge::initialize(&mut mapper);
        mapper
    }

    fn new_mmc3_nec() -> Mmc3Nec {
        let mut mapper = Mmc3Nec::new(test_data(4), false);
        Cartridge::initialize(&mut mapper);
        mapper
    }

    fn new_mmc6() -> Mmc6 {
        let mut mapper = Mmc6::new(test_data(1));
        Cartridge::initialize(&mut mapper);
        mapper
    }

    #[test]
    fn submapper0_resolves_to_standard_mmc3() {
        assert_eq!(
            resolve_mapper4_model(&test_data(0), None),
            Mapper4Model::Mmc3 {
                bus_conflicts: false
            }
        );
    }

    #[test]
    fn submapper1_resolves_to_mmc6() {
        assert_eq!(
            resolve_mapper4_model(&test_data(1), None),
            Mapper4Model::Mmc6
        );
    }

    #[test]
    fn submapper2_resolves_to_bus_conflict_mmc3() {
        assert_eq!(
            resolve_mapper4_model(&test_data(2), None),
            Mapper4Model::Mmc3 {
                bus_conflicts: true
            }
        );
    }

    #[test]
    fn submapper4_resolves_to_nec_mmc3() {
        assert_eq!(
            resolve_mapper4_model(&test_data(4), None),
            Mapper4Model::Mmc3Nec {
                bus_conflicts: false
            }
        );
    }

    #[test]
    fn sharp_override_forces_standard_mmc3() {
        assert_eq!(
            resolve_mapper4_model(&test_data(4), Some(Mmc3IrqVariant::Sharp)),
            Mapper4Model::Mmc3 {
                bus_conflicts: false
            }
        );
    }

    #[test]
    fn nec_override_forces_nec_mmc3() {
        assert_eq!(
            resolve_mapper4_model(&test_data(0), Some(Mmc3IrqVariant::Nec)),
            Mapper4Model::Mmc3Nec {
                bus_conflicts: false
            }
        );
    }

    #[test]
    fn standard_mmc3_uses_standard_irq_and_prg_ram_models() {
        let mapper = new_mmc3(0);
        assert_eq!(mapper.shared.irq_variant(), IrqVariant::Sharp);
        assert_eq!(mapper.shared.prg_ram_model(), PrgRamModel::Standard);
    }

    #[test]
    fn nec_mmc3_uses_old_style_irq_and_standard_prg_ram() {
        let mapper = new_mmc3_nec();
        assert_eq!(mapper.shared.irq_variant(), IrqVariant::NecOldStyle);
        assert_eq!(mapper.shared.prg_ram_model(), PrgRamModel::Standard);
    }

    #[test]
    fn mmc6_uses_old_style_irq_and_mmc6_prg_ram() {
        let mapper = new_mmc6();
        assert_eq!(mapper.shared.irq_variant(), IrqVariant::NecOldStyle);
        assert_eq!(mapper.shared.prg_ram_model(), PrgRamModel::Mmc6);
    }

    #[test]
    fn mmc6_maps_ram_at_7000_with_1kb_mirroring() {
        let mut mapper = new_mmc6();
        let mut interrupt = Interrupt::new();

        Mapper::write_register(&mut mapper, 0x8000, 0x20, &mut interrupt);
        Mapper::write_register(&mut mapper, 0xA001, 0xF0, &mut interrupt);
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
        let mut mapper = new_mmc6();
        let mut interrupt = Interrupt::new();

        Mapper::write_register(&mut mapper, 0xA001, 0xF0, &mut interrupt);
        assert_eq!(Mapper::read_ram(&mapper, 0x1000), None);

        Mapper::write_register(&mut mapper, 0x8000, 0x20, &mut interrupt);
        assert_eq!(Mapper::read_ram(&mapper, 0x1000), None);

        Mapper::write_register(&mut mapper, 0xA001, 0x30, &mut interrupt);
        Mapper::write_ram(&mut mapper, 0x1000, 0x56);
        Mapper::write_ram(&mut mapper, 0x1200, 0x78);

        assert_eq!(Mapper::read_ram(&mapper, 0x1000), Some(0x56));
        assert_eq!(Mapper::read_ram(&mapper, 0x1200), Some(0x00));

        Mapper::write_register(&mut mapper, 0x8000, 0x00, &mut interrupt);
        assert_eq!(Mapper::read_ram(&mapper, 0x1000), None);

        Mapper::write_register(&mut mapper, 0x8000, 0x20, &mut interrupt);
        assert_eq!(Mapper::read_ram(&mapper, 0x1000), None);
    }

    #[test]
    fn mmc6_cpu_6000_reads_as_open_bus_zero() {
        let mapper = new_mmc6();
        let read_result = Cartridge::read(&mapper, 0x6000);

        assert_eq!(read_result.data, 0);
        assert_eq!(read_result.mask, 0);
    }

    #[test]
    fn mmc6_does_not_retrigger_irq_when_reloading_after_natural_zero() {
        let mut mapper = new_mmc6();
        let mut interrupt = Interrupt::new();

        mapper.shared.set_irq_enabled(true);
        mapper.shared.write_irq_latch(1);
        mapper.shared.write_irq_reload();

        mapper.shared.clock_irq(&mut interrupt);
        assert_eq!(mapper.shared.irq_counter(), 1);
        assert!(!interrupt.get_irq(IrqSource::EXTERNAL));

        mapper.shared.write_irq_latch(0);
        mapper.shared.clock_irq(&mut interrupt);
        assert_eq!(mapper.shared.irq_counter(), 0);
        assert!(interrupt.get_irq(IrqSource::EXTERNAL));

        interrupt.clear_irq(IrqSource::EXTERNAL);
        mapper.shared.clock_irq(&mut interrupt);
        assert_eq!(mapper.shared.irq_counter(), 0);
        assert!(!interrupt.get_irq(IrqSource::EXTERNAL));
    }

    #[test]
    fn standard_mmc3_retriggers_irq_when_reloading_after_natural_zero() {
        let mut mapper = new_mmc3(0);
        let mut interrupt = Interrupt::new();

        mapper.shared.set_irq_enabled(true);
        mapper.shared.write_irq_latch(1);
        mapper.shared.write_irq_reload();

        mapper.shared.clock_irq(&mut interrupt);
        assert_eq!(mapper.shared.irq_counter(), 1);
        assert!(!interrupt.get_irq(IrqSource::EXTERNAL));

        mapper.shared.write_irq_latch(0);
        mapper.shared.clock_irq(&mut interrupt);
        assert_eq!(mapper.shared.irq_counter(), 0);
        assert!(interrupt.get_irq(IrqSource::EXTERNAL));

        interrupt.clear_irq(IrqSource::EXTERNAL);
        mapper.shared.clock_irq(&mut interrupt);
        assert_eq!(mapper.shared.irq_counter(), 0);
        assert!(interrupt.get_irq(IrqSource::EXTERNAL));
    }

    #[test]
    fn a12_rising_edge_below_filter_threshold_does_not_clock_irq() {
        let mut mapper = new_mmc3(0);
        let mut interrupt = Interrupt::new();
        mapper.shared.set_irq_counter(1);
        mapper.shared.set_irq_enabled(true);

        mapper.notify_ppu_bus_event(
            PpuBusEvent::AddressBusUpdate {
                address: 0x0FFF,
                ppu_tick: 0,
                from_cpu_register: true,
                access: PpuBusAccess::Read,
            },
            &mut interrupt,
        );
        mapper.notify_ppu_bus_event(
            PpuBusEvent::AddressBusUpdate {
                address: 0x1000,
                ppu_tick: 8,
                from_cpu_register: true,
                access: PpuBusAccess::Read,
            },
            &mut interrupt,
        );

        assert_eq!(mapper.shared.irq_counter(), 1);
        assert!(!interrupt.get_irq(IrqSource::EXTERNAL));
    }

    #[test]
    fn a12_rising_edge_at_filter_threshold_clocks_irq() {
        let mut mapper = new_mmc3(0);
        let mut interrupt = Interrupt::new();
        mapper.shared.set_irq_counter(1);
        mapper.shared.set_irq_enabled(true);

        mapper.notify_ppu_bus_event(
            PpuBusEvent::AddressBusUpdate {
                address: 0x0FFF,
                ppu_tick: 0,
                from_cpu_register: false,
                access: PpuBusAccess::Read,
            },
            &mut interrupt,
        );
        mapper.notify_ppu_bus_event(
            PpuBusEvent::AddressBusUpdate {
                address: 0x1000,
                ppu_tick: 9,
                from_cpu_register: false,
                access: PpuBusAccess::Read,
            },
            &mut interrupt,
        );

        assert_eq!(mapper.shared.irq_counter(), 0);
        assert!(interrupt.get_irq(IrqSource::EXTERNAL));
    }

    #[test]
    fn register_changes_still_require_filtered_a12_low_time() {
        let mut mapper = new_mmc3(0);
        let mut interrupt = Interrupt::new();
        mapper.shared.set_irq_counter(1);
        mapper.shared.set_irq_enabled(true);

        mapper.notify_ppu_bus_event(
            PpuBusEvent::AddressBusUpdate {
                address: 0x0FFF,
                ppu_tick: 0,
                from_cpu_register: true,
                access: PpuBusAccess::Read,
            },
            &mut interrupt,
        );
        mapper.notify_ppu_bus_event(
            PpuBusEvent::AddressBusUpdate {
                address: 0x1000,
                ppu_tick: 8,
                from_cpu_register: true,
                access: PpuBusAccess::Read,
            },
            &mut interrupt,
        );

        assert_eq!(mapper.shared.irq_counter(), 1);
        assert!(!interrupt.get_irq(IrqSource::EXTERNAL));

        mapper.notify_ppu_bus_event(
            PpuBusEvent::AddressBusUpdate {
                address: 0x0FFF,
                ppu_tick: 9,
                from_cpu_register: true,
                access: PpuBusAccess::Read,
            },
            &mut interrupt,
        );
        mapper.notify_ppu_bus_event(
            PpuBusEvent::AddressBusUpdate {
                address: 0x1000,
                ppu_tick: 18,
                from_cpu_register: true,
                access: PpuBusAccess::Read,
            },
            &mut interrupt,
        );

        assert_eq!(mapper.shared.irq_counter(), 0);
        assert!(interrupt.get_irq(IrqSource::EXTERNAL));
    }
}
