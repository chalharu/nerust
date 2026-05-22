// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

mod validation;

use self::validation::ValidationRunner;
use crate::manifest::{RomCase, read_rom};
use crate::results::{CaseOutcome, ValidationOptions};

pub fn validate_case(case: &RomCase, options: ValidationOptions) -> CaseOutcome {
    match read_rom(case)
        .and_then(|rom_bytes| ValidationRunner::new(case, &rom_bytes, options)?.run_case(case))
    {
        Ok(validation) => CaseOutcome::Completed(validation),
        Err(error) => CaseOutcome::InternalError {
            case_id: case.id.clone(),
            category: case.category,
            description: case.description.clone(),
            rom: case.rom.clone(),
            message: error.to_string(),
        },
    }
}
