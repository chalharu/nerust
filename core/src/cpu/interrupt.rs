// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

bitflags! {
    #[derive(Serialize, Deserialize)]
    pub(crate) struct IrqSource: u8 {
        const EXTERNAL = 0b0000_0001;
        const FRAME_COUNTER = 0b0000_0010;
        const DMC = 0b0000_0100;
        const FDS_DISK = 0b0000_1000;
        const ALL = 0xFF;
    }
}

#[derive(Serialize, Deserialize, Debug)]
pub(crate) struct Interrupt {
    pub nmi: bool,
    pub executing: bool,
    pub detected: bool,
    pub running_dma: bool,
    pub irq_mask: IrqSource,
    pub irq_flag: IrqSource,

    pub oam_dma: Option<u8>,
    pub dmc_start: bool,
    pub dmc_count: u8,
    pub write: bool,
}

impl Interrupt {
    pub fn new() -> Self {
        Self {
            nmi: false,
            executing: false,
            detected: false,
            running_dma: false,
            irq_mask: IrqSource::empty(),
            irq_flag: IrqSource::empty(),
            oam_dma: None,
            write: false,
            dmc_start: false,
            dmc_count: 0,
        }
    }

    pub fn set_irq(&mut self, source: IrqSource) {
        self.irq_flag |= source;
    }

    pub fn get_irq(&mut self, source: IrqSource) -> bool {
        !(self.irq_flag & source).is_empty()
    }

    pub fn clear_irq(&mut self, source: IrqSource) {
        self.irq_flag &= !source;
    }

    pub fn reset(&mut self) {
        self.nmi = false;
        self.executing = false;
        self.detected = false;
        self.running_dma = false;
        self.oam_dma = None;
        self.write = false;
        self.dmc_start = false;
        self.dmc_count = 0;
    }
}
