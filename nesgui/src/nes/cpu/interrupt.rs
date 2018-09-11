// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

#[derive(Serialize, Deserialize, Debug)]
pub(crate) struct Interrupt {
    pub irq: bool,
    pub reset: bool,
    pub nmi: bool,
}

impl Interrupt {
    pub fn new() -> Self {
        Self {
            irq: false,
            reset: true,
            nmi: false,
        }
    }

    pub fn set_reset(&mut self) {
        self.irq = false;
        self.nmi = false;
        self.reset = true;
    }

    pub fn set_nmi(&mut self) {
        self.nmi = true;
    }

    pub fn set_irq(&mut self) {
        self.irq = true;
    }

    pub fn reset_irq(&mut self) {
        self.irq = false;
    }

    pub fn reset_nmi(&mut self) {
        self.nmi = false;
    }

    pub fn reset_reset(&mut self) {
        self.reset = false;
    }
}
