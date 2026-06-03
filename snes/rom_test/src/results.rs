// Copyright (c) 2026 chalharu
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ValidationOptions {
    pub capture_screenshot_png: bool,
}

impl ValidationOptions {
    pub const fn testing() -> Self {
        Self {
            capture_screenshot_png: false,
        }
    }

    pub const fn report() -> Self {
        Self {
            capture_screenshot_png: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Validation {
    pub case_id: String,
    pub description: String,
    pub rom: String,
    pub steps_executed: u64,
    pub final_screen_hash: u64,
    pub screenshot_png: Option<Vec<u8>>,
    pub failures: Vec<String>,
}

impl Validation {
    pub fn passed(&self) -> bool {
        self.failures.is_empty()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CaseOutcome {
    Completed(Validation),
    InternalError {
        case_id: String,
        description: String,
        rom: String,
        message: String,
    },
}

impl CaseOutcome {
    pub fn passed(&self) -> bool {
        matches!(self, Self::Completed(validation) if validation.passed())
    }
}
