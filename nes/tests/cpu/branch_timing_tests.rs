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
        ScenarioLeaf::check_screen(25, 0x081BA42EB6C3294D)
    );
}

#[test]
fn _2_backward_branch() {
    test!(
        "cpu/branch_timing_tests/2.Backward_Branch.nes",
        ScenarioLeaf::check_screen(25, 0xE70FF858A009593F)
    );
}

#[test]
fn _3_forward_branch() {
    test!(
        "cpu/branch_timing_tests/3.Forward_Branch.nes",
        ScenarioLeaf::check_screen(25, 0xD394B778636B1CEF)
    );
}
