// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use super::CartridgeData;
use super::shared::{Mapper4Config, Mapper4Shared, Mapper4Wrapper};
use crate::cart_device::Cartridge;

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
impl Cartridge for TxSrom {}

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

    fn test_data() -> CartridgeData {
        let mut rom = vec![
            0x4E, 0x45, 0x53, 0x1A, 0x02, 0x01, 0x60, 0x78, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00,
        ];
        rom.resize(16 + 0x8000 + 0x2000, 0);
        CartridgeData::try_from(&mut rom.into_iter()).expect("cartridge data should parse")
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
}
