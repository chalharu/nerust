// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use super::CartridgeData;
use super::shared::{Mapper4Config, Mapper4Shared, Mapper4Wrapper};
use crate::cart_device::Cartridge;
use crate::persistence::{CartridgeRuntimeMessage, MAPPER_KIND_MMC3, PersistenceError};

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
    fn export_runtime_proto(&self) -> Result<CartridgeRuntimeMessage, PersistenceError> {
        Ok(CartridgeRuntimeMessage {
            mapper_state: Some(self.shared.export_state_proto()),
            mapper_specific_kind: MAPPER_KIND_MMC3.into(),
            mapper_specific_body: self.shared.export_runtime_body()?,
        })
    }

    fn import_runtime_proto(
        &mut self,
        payload: &CartridgeRuntimeMessage,
    ) -> Result<(), PersistenceError> {
        self.shared
            .import_state_proto(payload.mapper_state.as_ref().ok_or_else(|| {
                PersistenceError::Validation("missing MMC3 NEC mapper state".into())
            })?)?;
        if payload.mapper_specific_kind != MAPPER_KIND_MMC3 {
            return Err(PersistenceError::Validation(
                "unexpected MMC3 NEC runtime kind".into(),
            ));
        }
        self.shared
            .import_runtime_body(&payload.mapper_specific_body)
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
