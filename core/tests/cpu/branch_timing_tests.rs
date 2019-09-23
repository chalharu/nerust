// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use super::*;

#[test]
fn _1_branch_basics() {
    run_test!(
        "cpu/branch_timing_tests/1.Branch_Basics.nes",
        ScenarioLeaf::check_screen(25, 0x93D4_88FA_BC5E_367D)
    );
}

#[test]
fn _2_backward_branch() {
    run_test!(
        "cpu/branch_timing_tests/2.Backward_Branch.nes",
        ScenarioLeaf::check_screen(25, 0x3C97_7E76_7E7B_F4AD)
    );
}

#[test]
fn _3_forward_branch() {
    run_test!(
        "cpu/branch_timing_tests/3.Forward_Branch.nes",
        ScenarioLeaf::check_screen(25, 0x4F3A_6AA2_964D_33B7)
    );
}
