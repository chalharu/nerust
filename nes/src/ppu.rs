// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

struct Memory {}

pub struct Core {}

impl Core {
    pub fn new() -> Self {
        Self {
            // TODO:
        }
    }

    pub fn read_register(&mut self, address: usize) -> u8 {
        0
        // TODO:
    }

    pub fn write_register(&mut self, address: usize, value: u8) {
        // TODO:
    }
}
