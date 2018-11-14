// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use super::*;

#[test]
fn _1_branch_basics() {
    test!(
        "cpu/branch_timing_tests/1.Branch_Basics.nes",
        ScenarioLeaf::check_screen(25, 0x93D488FABC5E367D)
    );
}

#[test]
fn _2_backward_branch() {
    test!(
        "cpu/branch_timing_tests/2.Backward_Branch.nes",
        ScenarioLeaf::check_screen(25, 0x3C977E767E7BF4AD)
    );
}

#[test]
fn _3_forward_branch() {
    test!(
        "cpu/branch_timing_tests/3.Forward_Branch.nes",
        ScenarioLeaf::check_screen(25, 0x4F3A6AA2964D33B7)
    );
}
