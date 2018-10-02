// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

#[derive(Serialize, Deserialize, Debug)]
pub(crate) struct Interrupt {
    pub nmi: bool,
    pub executing: bool,
    pub detected: bool,
    pub running_dma: bool,
    pub irq_mask: u8,
    pub irq_flag: u8,

    pub oam_dma: Option<u8>,
}

impl Interrupt {
    pub fn new() -> Self {
        Self {
            nmi: false,
            executing: false,
            detected: false,
            running_dma: false,
            irq_mask: 0,
            irq_flag: 0,
            oam_dma: None,
        }
    }
}
