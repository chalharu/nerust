// Copyright (c) 2026 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Validation {
    pub case_id: String,
    pub steps_executed: u64,
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
    InternalError { case_id: String, message: String },
}
