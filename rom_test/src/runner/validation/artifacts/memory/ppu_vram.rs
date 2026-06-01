// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use super::super::super::runtime::ValidationRuntime;
use super::super::ValidationArtifacts;
use crate::error::RomTestError;
use crate::results::{PpuVramCheck, ValidationOptions};

#[derive(Default)]
pub(in crate::runner::validation::artifacts) struct PpuVramArtifacts {
    pub(in crate::runner::validation::artifacts) checks: Vec<PpuVramCheck>,
}

impl ValidationArtifacts {
    pub(in crate::runner::validation) fn record_ppu_vram_assert(
        &mut self,
        case_id: &str,
        runtime: &ValidationRuntime,
        options: ValidationOptions,
        frame: u64,
        address: usize,
        expected_value: u8,
    ) -> Result<(), RomTestError> {
        let actual_value = runtime.peek_ppu_vram(address).ok_or_else(|| {
            RomTestError::InvalidManifest(format!(
                "ROM case `{case_id}` requested check_ppu_vram outside PPU nametable/palette space at address 0x{address:04X}",
            ))
        })?;
        if options.check_expectations && actual_value != expected_value {
            self.failures.push(format!(
                "{case_id}: PPU VRAM mismatch at frame {frame} address 0x{address:04X} (expected 0x{expected_value:02X}, actual 0x{actual_value:02X})",
            ));
        }

        self.memory.ppu_vram.checks.push(PpuVramCheck {
            frame,
            address: u16::try_from(address).expect("address range validated before dispatch"),
            expected_value,
            actual_value,
        });
        Ok(())
    }
}
