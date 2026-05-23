// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

mod artifacts;
mod harness_impl;
mod runtime;

use self::artifacts::ValidationArtifacts;
use self::runtime::ValidationRuntime;
use crate::error::RomTestError;
use crate::harness::drive_case;
use crate::manifest::RomCase;
use crate::results::{CaseValidation, ValidationOptions};

pub(super) struct ValidationRunner {
    case_id: String,
    runtime: ValidationRuntime,
    artifacts: ValidationArtifacts,
    options: ValidationOptions,
}

impl ValidationRunner {
    pub(super) fn new(
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

    pub(super) fn run_case(mut self, case: &RomCase) -> Result<CaseValidation, RomTestError> {
        let totals = drive_case(case, &mut self)?;
        Ok(self
            .artifacts
            .finish(case, &self.runtime, totals, self.options))
    }
}
