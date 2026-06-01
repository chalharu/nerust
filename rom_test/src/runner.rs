// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

mod entry;
mod validation;

use crate::manifest::RomCase;
use crate::results::{CaseOutcome, ValidationOptions};

pub fn validate_case(case: &RomCase, options: ValidationOptions) -> CaseOutcome {
    entry::validate_case(case, options)
}
