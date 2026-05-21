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
pub(super) struct Mmc3Nec {
    pub(super) shared: Mapper4Shared,
}

impl Mmc3Nec {
    pub(super) fn new(data: CartridgeData, bus_conflicts: bool) -> Self {
        Self {
            shared: Mapper4Shared::new(data, Mapper4Config::mmc3_nec(bus_conflicts)),
        }
    }
}

#[typetag::serde]
impl Cartridge for Mmc3Nec {
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

impl Mapper4Wrapper for Mmc3Nec {
    const NAME: &'static str = "MMC3 NEC (Mapper4)";

    fn shared_ref(&self) -> &Mapper4Shared {
        &self.shared
    }

    fn shared_mut(&mut self) -> &mut Mapper4Shared {
        &mut self.shared
    }
}
