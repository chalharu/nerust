// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use super::manifest::{AudioExpectation, RomCategory};

#[derive(Debug, Clone, Copy)]
pub struct ValidationOptions {
    pub capture_screenshots: bool,
    pub check_expectations: bool,
}

impl ValidationOptions {
    pub const fn capturing() -> Self {
        Self {
            capture_screenshots: true,
            check_expectations: false,
        }
    }

    pub const fn report() -> Self {
        Self {
            capture_screenshots: true,
            check_expectations: true,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct ExecutionTotals {
    pub frames: u64,
    pub steps: u64,
}

#[derive(Debug, Clone)]
pub struct ScreenCheck {
    pub frame: u64,
    pub expected_hash: u64,
    pub actual_hash: u64,
    pub screenshot_png: Option<Vec<u8>>,
}

impl ScreenCheck {
    pub fn passed(&self) -> bool {
        self.expected_hash == self.actual_hash
    }
}

#[derive(Debug, Clone)]
pub struct WorkRamCheck {
    pub frame: u64,
    pub address: u16,
    pub expected_value: u8,
    pub actual_value: u8,
}

impl WorkRamCheck {
    pub fn passed(&self) -> bool {
        self.expected_value == self.actual_value
    }
}

#[derive(Debug, Clone)]
pub struct CartridgeRamCheck {
    pub frame: u64,
    pub address: u16,
    pub expected_value: u8,
    pub actual_value: u8,
    pub expected_open_bus: bool,
    pub actual_open_bus: bool,
}

impl CartridgeRamCheck {
    pub fn passed(&self) -> bool {
        self.expected_open_bus == self.actual_open_bus
            && (self.expected_open_bus || self.expected_value == self.actual_value)
    }
}

#[derive(Debug, Clone)]
pub struct PpuVramCheck {
    pub frame: u64,
    pub address: u16,
    pub expected_value: u8,
    pub actual_value: u8,
}

impl PpuVramCheck {
    pub fn passed(&self) -> bool {
        self.expected_value == self.actual_value
    }
}

#[derive(Debug, Clone)]
pub struct AudioObservation {
    pub sample_rate: u32,
    pub samples: u64,
    pub hash: u64,
    pub expected: Option<AudioExpectation>,
}

#[derive(Debug, Clone)]
pub struct CaseValidation {
    pub case_id: String,
    pub category: RomCategory,
    pub description: String,
    pub rom: String,
    pub frames: u64,
    pub steps: u64,
    pub final_screen_hash: u64,
    pub screen_checks: Vec<ScreenCheck>,
    pub work_ram_checks: Vec<WorkRamCheck>,
    pub cartridge_ram_checks: Vec<CartridgeRamCheck>,
    pub ppu_vram_checks: Vec<PpuVramCheck>,
    pub audio: AudioObservation,
    pub failures: Vec<String>,
}

impl CaseValidation {
    pub fn passed(&self) -> bool {
        self.failures.is_empty()
    }
}

#[derive(Debug, Clone)]
pub enum CaseOutcome {
    Completed(CaseValidation),
    InternalError {
        case_id: String,
        category: RomCategory,
        description: String,
        rom: String,
        message: String,
    },
}

impl CaseOutcome {
    pub fn case_id(&self) -> &str {
        match self {
            CaseOutcome::Completed(validation) => &validation.case_id,
            CaseOutcome::InternalError { case_id, .. } => case_id,
        }
    }

    pub fn category(&self) -> RomCategory {
        match self {
            CaseOutcome::Completed(validation) => validation.category,
            CaseOutcome::InternalError { category, .. } => *category,
        }
    }

    pub fn passed(&self) -> bool {
        match self {
            CaseOutcome::Completed(validation) => validation.passed(),
            CaseOutcome::InternalError { .. } => false,
        }
    }
}
