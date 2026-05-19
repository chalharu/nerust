// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use super::CartridgeData;
use super::shared::{Mapper4Config, Mapper4Shared, Mapper4Wrapper};
use crate::cart_device::Cartridge;

#[derive(serde_derive::Serialize, serde_derive::Deserialize)]
pub(super) struct Mmc6 {
    pub(super) shared: Mapper4Shared,
}

impl Mmc6 {
    pub(super) fn new(data: CartridgeData) -> Self {
        Self {
            shared: Mapper4Shared::new(data, Mapper4Config::mmc6()),
        }
    }
}

#[typetag::serde]
impl Cartridge for Mmc6 {}

impl Mapper4Wrapper for Mmc6 {
    const NAME: &'static str = "MMC6 (Mapper4)";

    fn shared_ref(&self) -> &Mapper4Shared {
        &self.shared
    }

    fn shared_mut(&mut self) -> &mut Mapper4Shared {
        &mut self.shared
    }
}
