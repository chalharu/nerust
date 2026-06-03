// Copyright (c) 2026 chalharu
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use nerust_contract_options::CoreOptions;
use nerust_contract_rom::RomIdentity;
use nerust_input_schema::SystemId;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CanonicalMediaIdentity {
    Rom(RomIdentity),
}

impl CanonicalMediaIdentity {
    pub const fn rom(rom_identity: RomIdentity) -> Self {
        Self::Rom(rom_identity)
    }

    pub const fn rom_identity(self) -> RomIdentity {
        match self {
            Self::Rom(rom_identity) => rom_identity,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PersistenceIdentity {
    pub system_id: SystemId,
    pub media: CanonicalMediaIdentity,
}

impl PersistenceIdentity {
    pub const fn rom(system_id: SystemId, rom_identity: RomIdentity) -> Self {
        Self {
            system_id,
            media: CanonicalMediaIdentity::Rom(rom_identity),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StateCompatibility {
    pub rom_identity: RomIdentity,
    pub options: CoreOptions,
}
