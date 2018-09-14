// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use core::collections::HashMap;

#[derive(Serialize, Deserialize, Debug, Copy, Clone)]
pub(crate) enum InterruptStatus {
    Polling,
    Detected,
    Executing,
}

#[derive(Serialize, Deserialize, Debug, Copy, Clone, Eq, PartialEq, Hash)]
pub(crate) enum IrqReason {
    ApuFrameCounter,
    ApuDmc,
}

#[derive(Serialize, Deserialize, Debug, Copy, Clone, Eq, PartialEq, Hash)]
pub(crate) enum IrqStatus {
    Acknowledge,
    Enabled,
    Used,
    Initialized,
}

#[derive(Serialize, Deserialize, Debug)]
pub(crate) struct Interrupt {
    irq_set: HashMap<IrqReason, IrqStatus>,
    pub reset: bool,
    pub nmi: bool,
    pub started: InterruptStatus,
}

impl Interrupt {
    pub fn new() -> Self {
        Self {
            irq_set: HashMap::new(),
            reset: true,
            nmi: false,
            started: InterruptStatus::Polling,
        }
    }

    pub fn set_nmi(&mut self) {
        self.nmi = true;
    }

    pub fn enable_irq(&mut self, reason: IrqReason) {
        self.irq_set.entry(reason).or_insert(IrqStatus::Initialized);
    }

    pub fn set_irq(&mut self, reason: IrqReason) {
        if let Some(entry) = self.irq_set.get_mut(&reason) {
            if *entry == IrqStatus::Acknowledge {
                *entry = IrqStatus::Enabled;
            }
        }
    }

    pub fn disable_irq(&mut self, reason: IrqReason) {
        self.irq_set.remove(&reason);
    }

    pub fn acknowledge_irq(&mut self, reason: IrqReason) {
        if let Some(entry) = self.irq_set.get_mut(&reason) {
            if *entry == IrqStatus::Used
                || *entry == IrqStatus::Enabled
                || *entry == IrqStatus::Initialized
            {
                *entry = IrqStatus::Acknowledge;
            }
        }
    }

    pub fn get_irq(&mut self) -> bool {
        self.irq_set.iter().any(|(_, &v)| v == IrqStatus::Enabled)
    }

    pub fn get_irq_with_reason(&mut self, reason: IrqReason) -> bool {
        if let Some(&status) = self.irq_set.get(&reason) {
            status == IrqStatus::Used || status == IrqStatus::Enabled
        } else {
            false
        }
    }

    pub fn use_irq(&mut self) {
        for (_, v) in self.irq_set.iter_mut() {
            if *v == IrqStatus::Enabled {
                *v = IrqStatus::Used;
            }
        }
    }

    // pub fn reset_irq(&mut self) {
    //     self.irq_set.clear();
    // }

    pub fn unset_reset(&mut self) {
        self.reset = false;
    }
}
