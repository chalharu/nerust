// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use super::artifacts::ValidationArtifacts;
use super::assertions::CartridgeRamAssertion;
use super::runtime::ValidationRuntime;
use crate::error::RomTestError;
use crate::events::{ButtonCode, ControllerPad, PadState};
use crate::harness::drive_case;
use crate::manifest::RomCase;
use crate::results::{CaseValidation, ValidationOptions};

pub(in crate::runner) struct ValidationRunner {
    case_id: String,
    runtime: ValidationRuntime,
    artifacts: ValidationArtifacts,
    options: ValidationOptions,
}

impl ValidationRunner {
    pub(in crate::runner) fn new(
        case: &RomCase,
        rom_bytes: &[u8],
        options: ValidationOptions,
    ) -> Result<Self, RomTestError> {
        Ok(Self {
            case_id: case.id.clone(),
            runtime: ValidationRuntime::new(case, rom_bytes)?,
            artifacts: ValidationArtifacts::default(),
            options,
        })
    }

    pub(in crate::runner) fn run_case(
        mut self,
        case: &RomCase,
    ) -> Result<CaseValidation, RomTestError> {
        let totals = drive_case(case, &mut self)?;
        Ok(self
            .artifacts
            .finish(case, &self.runtime, totals, self.options))
    }

    pub(in crate::runner::validation) fn run_frame(&mut self) -> u64 {
        self.runtime.run_frame()
    }

    pub(in crate::runner::validation) fn frame_counter(&self) -> u64 {
        self.runtime.frame_counter()
    }

    pub(in crate::runner::validation) fn record_screen_assert(
        &mut self,
        frame: u64,
        expected_hash: u64,
    ) -> Result<(), RomTestError> {
        self.artifacts.record_screen_assert(
            &self.case_id,
            &self.runtime,
            self.options,
            frame,
            expected_hash,
        )
    }

    pub(in crate::runner::validation) fn record_work_ram_assert(
        &mut self,
        frame: u64,
        address: usize,
        expected_value: u8,
    ) -> Result<(), RomTestError> {
        self.artifacts.record_work_ram_assert(
            &self.case_id,
            &self.runtime,
            self.options,
            frame,
            address,
            expected_value,
        )
    }

    pub(in crate::runner::validation) fn record_cartridge_ram_assert(
        &mut self,
        frame: u64,
        address: usize,
        expected_value: u8,
        expect_open_bus: bool,
    ) -> Result<(), RomTestError> {
        self.artifacts.record_cartridge_ram_assert(
            &self.case_id,
            &self.runtime,
            self.options,
            CartridgeRamAssertion {
                frame,
                address,
                expected_value,
                expect_open_bus,
            },
        )
    }

    pub(in crate::runner::validation) fn record_ppu_vram_assert(
        &mut self,
        frame: u64,
        address: usize,
        expected_value: u8,
    ) -> Result<(), RomTestError> {
        self.artifacts.record_ppu_vram_assert(
            &self.case_id,
            &self.runtime,
            self.options,
            frame,
            address,
            expected_value,
        )
    }

    pub(in crate::runner::validation) fn reset_runtime(&mut self) {
        self.runtime.reset();
    }

    pub(in crate::runner::validation) fn apply_standard_controller(
        &mut self,
        pad: ControllerPad,
        button: ButtonCode,
        state: PadState,
    ) {
        self.runtime.apply_standard_controller(pad, button, state);
    }

    pub(in crate::runner::validation) fn set_microphone(&mut self, state: PadState) {
        self.runtime.set_microphone(state);
    }
}
