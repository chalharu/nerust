// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use crate::persistence::{InterruptMessage, PersistenceError};

bitflags::bitflags! {
    #[derive(
        serde_derive::Serialize,
        serde_derive::Deserialize,
        Debug,
        Clone,
        Copy,
    )]
    pub(crate) struct IrqSource: u8 {
        const EXTERNAL = 0b0000_0001;
        const FRAME_COUNTER = 0b0000_0010;
        const DMC = 0b0000_0100;
        const FDS_DISK = 0b0000_1000;
        const ALL = 0xFF;
    }
}

#[derive(serde_derive::Serialize, serde_derive::Deserialize, Debug, Copy, Clone, PartialEq, Eq)]
pub(crate) enum DmcDmaKind {
    Load,
    Reload,
}

#[derive(serde_derive::Serialize, serde_derive::Deserialize, Debug, Copy, Clone)]
pub(crate) struct Interrupt {
    pub nmi: bool,
    pub executing: bool,
    pub detected: bool,
    pub running_dma: bool,
    pub irq_mask: IrqSource,
    pub irq_flag: IrqSource,
    pub oam_dma: Option<u8>,
    pub dmc_dma_request: Option<DmcDmaKind>,
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
            dmc_dma_request: None,
            write: false,
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
        self.dmc_dma_request = None;
        self.write = false;
    }
}

impl Default for Interrupt {
    fn default() -> Self {
        Self::new()
    }
}

impl Interrupt {
    pub(crate) fn export_state_proto(&self) -> InterruptMessage {
        InterruptMessage {
            nmi: self.nmi,
            executing: self.executing,
            detected: self.detected,
            running_dma: self.running_dma,
            irq_mask: self.irq_mask.bits().into(),
            irq_flag: self.irq_flag.bits().into(),
            oam_dma: self.oam_dma.map(u32::from),
            dmc_dma_request: self.dmc_dma_request.map(|value| match value {
                DmcDmaKind::Load => 0,
                DmcDmaKind::Reload => 1,
            }),
            write: self.write,
        }
    }

    pub(crate) fn import_state_proto(
        &mut self,
        payload: &InterruptMessage,
    ) -> Result<(), PersistenceError> {
        self.nmi = payload.nmi;
        self.executing = payload.executing;
        self.detected = payload.detected;
        self.running_dma = payload.running_dma;
        self.irq_mask = IrqSource::from_bits(
            u8::try_from(payload.irq_mask)
                .map_err(|_| PersistenceError::Validation("IRQ mask overflow".into()))?,
        )
        .ok_or_else(|| PersistenceError::Validation("invalid IRQ mask".into()))?;
        self.irq_flag = IrqSource::from_bits(
            u8::try_from(payload.irq_flag)
                .map_err(|_| PersistenceError::Validation("IRQ flag overflow".into()))?,
        )
        .ok_or_else(|| PersistenceError::Validation("invalid IRQ flag".into()))?;
        self.oam_dma = payload
            .oam_dma
            .map(|value| {
                u8::try_from(value)
                    .map_err(|_| PersistenceError::Validation("OAM DMA overflow".into()))
            })
            .transpose()?;
        self.dmc_dma_request = match payload.dmc_dma_request {
            Some(0) => Some(DmcDmaKind::Load),
            Some(1) => Some(DmcDmaKind::Reload),
            Some(_) => {
                return Err(PersistenceError::Validation(
                    "invalid DMC DMA request".into(),
                ));
            }
            None => None,
        };
        self.write = payload.write;
        Ok(())
    }
}
