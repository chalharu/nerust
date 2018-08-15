// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

struct Memory {}

pub struct Core {}

use super::*;

impl Core {
    pub fn step(
        &mut self,
        ppu: &mut Ppu,
        cartridge: &mut Cartridge,
        controller: &mut Bus,
        apu: &mut Apu,
    ) {
    }

    pub fn new() -> Self {
        Self {}
    }
}
