// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

#[derive(Serialize, Deserialize, Debug)]
pub(crate) struct Interrupt {
    pub reset: bool,

    pub nmi: bool,
    pub executing: bool,
    pub detected: bool,
    pub running_dma: bool,
    pub irq_mask: u8,
    pub irq_flag: u8,
}

impl Interrupt {
    pub fn new() -> Self {
        Self {
            reset: true,
            nmi: false,
            executing: false,
            detected: false,
            running_dma: false,
            irq_mask: 0,
            irq_flag: 0,
        }
    }

    // pub fn reset_irq(&mut self) {
    //     self.irq_set.clear();
    // }

    pub fn set_reset(&mut self) {
        self.reset = true;
        self.executing = false;
        self.detected = false;
        self.nmi = false;
        self.running_dma = false;
        self.irq_flag = 0;
        self.irq_mask = 0xFF;
    }

    pub fn unset_reset(&mut self) {
        self.reset = false;
    }
}
