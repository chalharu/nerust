// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use super::runtime::ValidationRuntime;
use crate::error::RomTestError;
use crate::manifest::RomCase;
use crate::results::{
    AudioObservation, CartridgeRamCheck, CaseValidation, ExecutionTotals, PpuVramCheck,
    ScreenCheck, ValidationOptions, WorkRamCheck,
};

#[derive(Default)]
pub(super) struct ValidationArtifacts {
    screen_checks: Vec<ScreenCheck>,
    work_ram_checks: Vec<WorkRamCheck>,
    cartridge_ram_checks: Vec<CartridgeRamCheck>,
    ppu_vram_checks: Vec<PpuVramCheck>,
    failures: Vec<String>,
}

#[derive(Clone, Copy)]
pub(super) struct CartridgeRamAssertion {
    pub(super) frame: u64,
    pub(super) address: usize,
    pub(super) expected_value: u8,
    pub(super) expect_open_bus: bool,
}

impl ValidationArtifacts {
    pub(super) fn finish(
        mut self,
        case: &RomCase,
        runtime: &ValidationRuntime,
        totals: ExecutionTotals,
        options: ValidationOptions,
    ) -> CaseValidation {
        let final_screen_hash = runtime.screen_hash();
        let audio = AudioObservation {
            sample_rate: runtime.audio_sample_rate(),
            samples: runtime.audio_samples(),
            hash: runtime.audio_hash(),
            expected: case.expected_audio.clone(),
        };

        if options.check_expectations
            && let Some(expected_audio) = &audio.expected
        {
            if audio.samples != expected_audio.samples {
                self.failures.push(format!(
                    "{}: audio sample mismatch (expected {}, actual {})",
                    case.id, expected_audio.samples, audio.samples
                ));
            }
            if audio.hash != expected_audio.hash {
                self.failures.push(format!(
                    "{}: audio hash mismatch (expected 0x{:016X}, actual 0x{:016X})",
                    case.id, expected_audio.hash, audio.hash
                ));
            }
        }

        CaseValidation {
            case_id: case.id.clone(),
            category: case.category,
            description: case.description.clone(),
            rom: case.rom.clone(),
            frames: totals.frames,
            steps: totals.steps,
            final_screen_hash,
            screen_checks: self.screen_checks,
            work_ram_checks: self.work_ram_checks,
            cartridge_ram_checks: self.cartridge_ram_checks,
            ppu_vram_checks: self.ppu_vram_checks,
            audio,
            failures: self.failures,
        }
    }

    pub(super) fn record_screen_assert(
        &mut self,
        case_id: &str,
        runtime: &ValidationRuntime,
        options: ValidationOptions,
        frame: u64,
        expected_hash: u64,
    ) -> Result<(), RomTestError> {
        let actual_hash = runtime.screen_hash();
        if options.check_expectations && actual_hash != expected_hash {
            self.failures.push(format!(
                "{case_id}: screen hash mismatch at frame {frame} (expected 0x{expected_hash:016X}, actual 0x{actual_hash:016X})",
            ));
        }

        let screenshot_png = if options.capture_screenshots {
            Some(runtime.capture_screenshot_png()?)
        } else {
            None
        };

        self.screen_checks.push(ScreenCheck {
            frame,
            expected_hash,
            actual_hash,
            screenshot_png,
        });
        Ok(())
    }

    pub(super) fn record_work_ram_assert(
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

        self.work_ram_checks.push(WorkRamCheck {
            frame,
            address: u16::try_from(address).expect("address range validated before dispatch"),
            expected_value,
            actual_value,
        });
        Ok(())
    }

    pub(super) fn record_cartridge_ram_assert(
        &mut self,
        case_id: &str,
        runtime: &ValidationRuntime,
        options: ValidationOptions,
        assertion: CartridgeRamAssertion,
    ) -> Result<(), RomTestError> {
        let (actual_value, actual_open_bus) = runtime
            .peek_cartridge_ram(assertion.address)
            .ok_or_else(|| {
            RomTestError::InvalidManifest(format!(
                "ROM case `{case_id}` requested check_cartridge_ram outside cartridge RAM at address 0x{:04X}",
                assertion.address,
            ))
        })?;
        if options.check_expectations && actual_open_bus != assertion.expect_open_bus {
            self.failures.push(format!(
                "{case_id}: cartridge RAM bus state mismatch at frame {} address 0x{:04X} (expected {}, actual {})",
                assertion.frame,
                assertion.address,
                if assertion.expect_open_bus { "open bus" } else { "mapped RAM" },
                if actual_open_bus { "open bus" } else { "mapped RAM" }
            ));
        }
        if options.check_expectations
            && !assertion.expect_open_bus
            && actual_value != assertion.expected_value
        {
            self.failures.push(format!(
                "{case_id}: cartridge RAM mismatch at frame {} address 0x{:04X} (expected 0x{:02X}, actual 0x{:02X})",
                assertion.frame,
                assertion.address,
                assertion.expected_value,
                actual_value,
            ));
        }

        self.cartridge_ram_checks.push(CartridgeRamCheck {
            frame: assertion.frame,
            address: u16::try_from(assertion.address)
                .expect("address range validated before dispatch"),
            expected_value: assertion.expected_value,
            actual_value,
            expected_open_bus: assertion.expect_open_bus,
            actual_open_bus,
        });
        Ok(())
    }

    pub(super) fn record_ppu_vram_assert(
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

        self.ppu_vram_checks.push(PpuVramCheck {
            frame,
            address: u16::try_from(address).expect("address range validated before dispatch"),
            expected_value,
            actual_value,
        });
        Ok(())
    }
}
