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

use rom_test::{CaseOutcome, ValidationOptions, load_default_manifest, validate_case};
use std::sync::OnceLock;

#[test]
fn rom_manifest_is_well_formed() {
    let manifest = manifest();
    assert_eq!(
        GENERATED_ROM_CASE_COUNT,
        manifest.cases.len(),
        "generated test count should match the manifest case count"
    );
}

fn manifest() -> &'static rom_test::RomManifest {
    static MANIFEST: OnceLock<rom_test::RomManifest> = OnceLock::new();
    MANIFEST.get_or_init(|| load_default_manifest().expect("ROM manifest should load"))
}

fn run_generated_manifest_case(case_id: &str) {
    let case = manifest()
        .case(case_id)
        .unwrap_or_else(|| panic!("ROM case `{case_id}` should exist in the manifest"));
    let outcome = validate_case(
        case,
        ValidationOptions {
            capture_screenshots: false,
            check_expectations: true,
        },
    );

    match outcome {
        CaseOutcome::Completed(validation) if validation.passed() => {}
        CaseOutcome::Completed(validation) => {
            panic!(
                "{}:\n{}",
                validation.case_id,
                validation.failures.join("\n")
            );
        }
        CaseOutcome::InternalError {
            case_id, message, ..
        } => {
            panic!("{case_id}: {message}");
        }
    }
}

include!(concat!(env!("OUT_DIR"), "/generated_rom_manifest_tests.rs"));
