// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use super::CartridgeData;
use super::shared::{Mapper4Config, Mapper4Shared, Mapper4Wrapper};
use crate::OpenBusReadResult;
use crate::cart_device::Cartridge;
use crate::cpu::interrupt::Interrupt;
use crate::persistence::{CartridgeRuntimeState, PersistenceError};
use crate::ppu_memory_access::PpuReadAccess;

#[derive(serde_derive::Serialize, serde_derive::Deserialize)]
pub(super) struct TxSrom {
    pub(super) shared: Mapper4Shared,
}

impl TxSrom {
    pub(super) fn new(data: CartridgeData) -> Self {
        Self {
            shared: Mapper4Shared::new(data, Mapper4Config::txsrom()),
        }
    }
}

#[typetag::serde]
impl Cartridge for TxSrom {
    fn export_runtime_state(&self) -> Result<CartridgeRuntimeState, PersistenceError> {
        self.shared.export_runtime_state()
    }

    fn import_runtime_state(
        &mut self,
        state: CartridgeRuntimeState,
    ) -> Result<(), PersistenceError> {
        self.shared.import_runtime_state(state)
    }

    fn read_ppu_pattern(
        &mut self,
        address: usize,
        access: PpuReadAccess,
        interrupt: &mut Interrupt,
    ) -> OpenBusReadResult {
        self.shared.read_ppu_pattern(address, access, interrupt)
    }

    fn write_ppu_pattern(&mut self, address: usize, value: u8, interrupt: &mut Interrupt) {
        self.shared.write_ppu_pattern(address, value, interrupt);
    }
}

impl Mapper4Wrapper for TxSrom {
    const NAME: &'static str = "TxSROM (Mapper118)";

    fn shared_ref(&self) -> &Mapper4Shared {
        &self.shared
    }

    fn shared_mut(&mut self) -> &mut Mapper4Shared {
        &mut self.shared
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::MirrorMode;
    use crate::cart_device::Cartridge;
    use crate::cpu::interrupt::Interrupt;
    use crate::mapper::Mapper;
    use crate::persistence::{decode_payload, encode_payload};
    use crate::ppu_memory_access::PpuReadAccess;
    use crate::{CartridgeDataParts, RomFormat};

    fn test_data() -> CartridgeData {
        CartridgeData::new(CartridgeDataParts {
            format: RomFormat::Nes20,
            prog_rom: vec![0; 0x8000],
            char_rom: vec![0; 0x2000],
            pram_length: 0,
            save_pram_length: 0,
            vram_length: 0,
            save_vram_length: 0,
            mapper_type: 118,
            mirror_mode: MirrorMode::Four,
            has_battery: false,
            sub_mapper_type: 7,
            trainer: Vec::new(),
        })
        .expect("test cartridge data should be valid")
    }

    fn new_txsrom() -> TxSrom {
        let mut mapper = TxSrom::new(test_data());
        Cartridge::initialize(&mut mapper);
        mapper
    }

    #[test]
    fn bank_registers_0_and_1_control_two_kib_nametable_pairs_in_normal_chr_mode() {
        let mut mapper = new_txsrom();
        let mut interrupt = Interrupt::new();

        Mapper::write_register(&mut mapper, 0x8000, 0x00, &mut interrupt);
        Mapper::write_register(&mut mapper, 0x8001, 0x80, &mut interrupt);

        assert_eq!(
            Mapper::get_mirror_mode(&mapper),
            MirrorMode::Custom([1, 1, 0, 0])
        );
    }

    #[test]
    fn bank_registers_2_through_5_control_individual_nametables_in_inverted_chr_mode() {
        let mut mapper = new_txsrom();
        let mut interrupt = Interrupt::new();

        Mapper::write_register(&mut mapper, 0x8000, 0x82, &mut interrupt);
        Mapper::write_register(&mut mapper, 0x8001, 0x80, &mut interrupt);
        Mapper::write_register(&mut mapper, 0x8000, 0x84, &mut interrupt);
        Mapper::write_register(&mut mapper, 0x8001, 0x80, &mut interrupt);

        assert_eq!(
            Mapper::get_mirror_mode(&mapper),
            MirrorMode::Custom([1, 0, 1, 0])
        );
    }

    #[test]
    fn a000_mirroring_writes_do_not_override_txsrom_nametable_mapping() {
        let mut mapper = new_txsrom();
        let mut interrupt = Interrupt::new();

        Mapper::write_register(&mut mapper, 0x8000, 0x82, &mut interrupt);
        Mapper::write_register(&mut mapper, 0x8001, 0x80, &mut interrupt);
        Mapper::write_register(&mut mapper, 0xA000, 0x01, &mut interrupt);

        assert_eq!(
            Mapper::get_mirror_mode(&mapper),
            MirrorMode::Custom([1, 0, 0, 0])
        );
    }

    #[test]
    fn chr_register_write_updates_txsrom_mirroring_immediately() {
        let mut mapper = new_txsrom();
        let mut interrupt = Interrupt::new();

        assert_eq!(
            Mapper::get_mirror_mode(&mapper),
            MirrorMode::Custom([0, 0, 0, 0])
        );

        Mapper::write_register(&mut mapper, 0x8000, 0x00, &mut interrupt);
        Mapper::write_register(&mut mapper, 0x8001, 0x82, &mut interrupt);

        assert!(mapper.shared.pending_chr_update());
        assert!(!mapper.shared.chr_mapping_in_sync());
        assert_eq!(
            Mapper::get_mirror_mode(&mapper),
            MirrorMode::Custom([1, 1, 0, 0])
        );

        Cartridge::read_ppu_pattern(
            &mut mapper,
            0x0008,
            PpuReadAccess::BackgroundPattern,
            &mut interrupt,
        );
        assert_eq!(
            Mapper::get_mirror_mode(&mapper),
            MirrorMode::Custom([1, 1, 0, 0])
        );
        assert!(mapper.shared.pending_chr_update());

        Cartridge::read_ppu_pattern(
            &mut mapper,
            0x0000,
            PpuReadAccess::BackgroundPattern,
            &mut interrupt,
        );
        assert_eq!(
            Mapper::get_mirror_mode(&mapper),
            MirrorMode::Custom([1, 1, 0, 0])
        );
        assert!(!mapper.shared.pending_chr_update());
        assert!(mapper.shared.chr_mapping_in_sync());
    }

    #[test]
    fn current_payload_deserialization_preserves_txsrom_mirroring() {
        let mut mapper = new_txsrom();
        let mut interrupt = Interrupt::new();

        Mapper::write_register(&mut mapper, 0x8000, 0x00, &mut interrupt);
        Mapper::write_register(&mut mapper, 0x8001, 0x82, &mut interrupt);

        let encoded = encode_payload(&mapper).expect("mapper payload should encode");
        let decoded: TxSrom = decode_payload(&encoded).expect("mapper payload should decode");

        assert_eq!(
            Mapper::get_mirror_mode(&decoded),
            MirrorMode::Custom([1, 1, 0, 0])
        );
        assert!(decoded.shared.pending_chr_update());
        assert!(!decoded.shared.chr_mapping_in_sync());
    }
}
