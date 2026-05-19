// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use super::shared::{Mapper4Config, Mapper4Shared, Mapper4Wrapper};
use crate::cartridge_api::Cartridge;
use crate::cartridge_data::CartridgeData;

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
impl Cartridge for Mmc3Nec {}

impl Mapper4Wrapper for Mmc3Nec {
    const NAME: &'static str = "MMC3 NEC (Mapper4)";

    fn shared_ref(&self) -> &Mapper4Shared {
        &self.shared
    }

    fn shared_mut(&mut self) -> &mut Mapper4Shared {
        &mut self.shared
    }
}
