// Copyright (c) 2018 chalharu
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use super::super::super::runtime::ValidationRuntime;
use super::super::ValidationArtifacts;
use crate::error::RomTestError;
use crate::results::{ValidationOptions, WorkRamCheck};

#[derive(Default)]
pub(in crate::runner::validation::artifacts) struct WorkRamArtifacts {
    pub(in crate::runner::validation::artifacts) checks: Vec<WorkRamCheck>,
}

impl ValidationArtifacts {
    pub(in crate::runner::validation) fn record_work_ram_assert(
        &mut self,
        case_id: &str,
        runtime: &ValidationRuntime,
        options: ValidationOptions,
        frame: u64,
        address: usize,
        expected_value: u8,
    ) -> Result<(), RomTestError> {
        let actual_value = runtime.peek_work_ram(address).ok_or_else(|| {
            RomTestError::InvalidManifest(format!(
                "ROM case `{case_id}` requested check_work_ram outside CPU work RAM at address 0x{address:04X}",
            ))
        })?;
        if options.check_expectations && actual_value != expected_value {
            self.failures.push(format!(
                "{case_id}: work RAM mismatch at frame {frame} address 0x{address:04X} (expected 0x{expected_value:02X}, actual 0x{actual_value:02X})",
            ));
        }

        self.memory.work_ram.checks.push(WorkRamCheck {
            frame,
            address: u16::try_from(address).expect("address range validated before dispatch"),
            expected_value,
            actual_value,
        });
        Ok(())
    }
}
