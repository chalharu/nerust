// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use super::{CartridgeRamAssertion, ValidationRunner};
use crate::error::RomTestError;
use crate::events::{MemoryAssertionSpace, RomAssertion};
use crate::harness::CaseHarness;

impl CaseHarness for ValidationRunner {
    fn run_frame(&mut self) -> u64 {
        self.runtime.run_frame()
    }

    fn frame_counter(&self) -> u64 {
        self.runtime.frame_counter()
    }

    fn on_assert(&mut self, frame: u64, assertion: &RomAssertion) -> Result<(), RomTestError> {
        match assertion {
            RomAssertion::Screen { hash } => self.artifacts.record_screen_assert(
                &self.case_id,
                &self.runtime,
                self.options,
                frame,
                *hash,
            ),
            RomAssertion::Memory {
                space,
                address,
                value,
                open_bus,
            } => match space {
                MemoryAssertionSpace::WorkRam => self.artifacts.record_work_ram_assert(
                    &self.case_id,
                    &self.runtime,
                    self.options,
                    frame,
                    usize::from(*address),
                    *value,
                ),
                MemoryAssertionSpace::CartridgeRam => self.artifacts.record_cartridge_ram_assert(
                    &self.case_id,
                    &self.runtime,
                    self.options,
                    CartridgeRamAssertion {
                        frame,
                        address: usize::from(*address),
                        expected_value: *value,
                        expect_open_bus: *open_bus,
                    },
                ),
                MemoryAssertionSpace::PpuVram => self.artifacts.record_ppu_vram_assert(
                    &self.case_id,
                    &self.runtime,
                    self.options,
                    frame,
                    usize::from(*address),
                    *value,
                ),
            },
        }
    }

    fn on_reset(&mut self) -> Result<(), RomTestError> {
        self.runtime.reset();
        Ok(())
    }

    fn on_standard_controller(
        &mut self,
        pad: crate::events::ControllerPad,
        button: crate::events::ButtonCode,
        state: crate::events::PadState,
    ) -> Result<(), RomTestError> {
        self.runtime.apply_standard_controller(pad, button, state);
        Ok(())
    }

    fn on_microphone(&mut self, state: crate::events::PadState) -> Result<(), RomTestError> {
        self.runtime.set_microphone(state);
        Ok(())
    }
}
