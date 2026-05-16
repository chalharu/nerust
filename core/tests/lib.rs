// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

#[allow(
    dead_code,
    reason = "the integration test links the shared ROM tooling module but does not exercise every helper"
)]
#[path = "../src/rom_test.rs"]
mod rom_test;

use rom_test::{
    CaseOutcome, ValidationOptions, default_output_root, load_default_manifest, validate_case,
    write_html_report,
};

#[test]
fn rom_manifest_is_well_formed() {
    load_default_manifest().expect("ROM manifest should load");
}

#[test]
fn rom_manifest_cases_validate() {
    let manifest = load_default_manifest().expect("ROM manifest should load");
    let outcomes = manifest
        .cases
        .iter()
        .map(|case| validate_case(case, ValidationOptions::report()))
        .collect::<Vec<_>>();
    let report = write_html_report(
        &default_output_root().join("test-validate"),
        "ROM validation report",
        &outcomes,
    )
    .expect("ROM validation report should be written");
    let failures = outcomes
        .iter()
        .filter_map(|outcome| match outcome {
            CaseOutcome::Completed(validation) if validation.passed() => None,
            CaseOutcome::Completed(validation) => Some(format!(
                "{}:\n{}",
                outcome.case_id(),
                validation.failures.join("\n")
            )),
            CaseOutcome::InternalError { message, .. } => {
                Some(format!("{}: {message}", outcome.case_id()))
            }
        })
        .collect::<Vec<_>>();

    if !failures.is_empty() {
        panic!(
            "ROM validation failures ({}):\n{}",
            failures.len(),
            failures.join("\n\n")
        );
    }

    assert_eq!(
        report.failed, 0,
        "ROM validation report should have no failures"
    );
    assert_eq!(
        report.passed,
        manifest.cases.len(),
        "ROM validation report should cover every manifest case"
    );
    assert!(
        report.report_path.is_file(),
        "ROM validation report should be generated at {}",
        report.report_path.display()
    );
}
