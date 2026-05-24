// Copyright (c) 2026 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use nerust_contract_options::CoreOptions;
use nerust_contract_rom::RomIdentity;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PersistenceTarget {
    pub rom_identity: RomIdentity,
    pub options: CoreOptions,
}
