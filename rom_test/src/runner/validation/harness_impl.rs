// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use super::ValidationRunner;
use crate::error::RomTestError;
use crate::events::{MemoryAssertionSpace, RomAssertion};
use crate::harness::CaseHarness;

impl CaseHarness for ValidationRunner {
    fn run_frame(&mut self) -> u64 {
        self.run_frame()
    }

    fn frame_counter(&self) -> u64 {
        self.frame_counter()
    }

    fn on_assert(&mut self, frame: u64, assertion: &RomAssertion) -> Result<(), RomTestError> {
        match assertion {
            RomAssertion::Screen { hash } => self.record_screen_assert(frame, *hash),
            RomAssertion::Memory {
                space,
                address,
                value,
                open_bus,
            } => match space {
                MemoryAssertionSpace::WorkRam => {
                    self.record_work_ram_assert(frame, usize::from(*address), *value)
                }
                MemoryAssertionSpace::CartridgeRam => self.record_cartridge_ram_assert(
                    frame,
                    usize::from(*address),
                    *value,
                    *open_bus,
                ),
                MemoryAssertionSpace::PpuVram => {
                    self.record_ppu_vram_assert(frame, usize::from(*address), *value)
                }
            },
        }
    }

    fn on_reset(&mut self) -> Result<(), RomTestError> {
        self.reset_runtime();
        Ok(())
    }

    fn on_standard_controller(
        &mut self,
        pad: crate::events::ControllerPad,
        button: crate::events::ButtonCode,
        state: crate::events::PadState,
    ) -> Result<(), RomTestError> {
        self.apply_standard_controller(pad, button, state);
        Ok(())
    }

    fn on_microphone(&mut self, state: crate::events::PadState) -> Result<(), RomTestError> {
        self.set_microphone(state);
        Ok(())
    }
}
