// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

#[path = "../src/rom_test.rs"]
mod rom_test;

use rom_test::{CaseOutcome, ValidationOptions, load_default_manifest, validate_case};

#[test]
fn rom_manifest_is_well_formed() {
    load_default_manifest().expect("ROM manifest should load");
}

#[test]
fn rom_manifest_cases_validate() {
    let manifest = load_default_manifest().expect("ROM manifest should load");
    let mut failures = Vec::new();

    for case in &manifest.cases {
        match validate_case(case, ValidationOptions::validating()) {
            CaseOutcome::Completed(validation) if validation.passed() => {}
            CaseOutcome::Completed(validation) => {
                failures.push(format!(
                    "{}:\n{}",
                    validation.case_id,
                    validation.failures.join("\n")
                ));
            }
            CaseOutcome::InternalError {
                case_id, message, ..
            } => failures.push(format!("{case_id}: {message}")),
        }
    }

    if !failures.is_empty() {
        panic!(
            "ROM validation failures ({}):\n{}",
            failures.len(),
            failures.join("\n\n")
        );
    }
}
