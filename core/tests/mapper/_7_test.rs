// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use super::*;

#[test]
fn _7_test_0() {
    run_test!(
        "mapper/7_test_src/7_test_0.nes",
        ScenarioLeaf::check_screen(50, 0x29DF_181B_7DD6_EEA1)
    );
}

#[test]
fn _7_test_1() {
    run_test!(
        "mapper/7_test_src/7_test_1.nes",
        ScenarioLeaf::check_screen(50, 0x29DF_181B_7DD6_EEA1)
    );
}

#[test]
fn _7_test_2() {
    run_test!(
        "mapper/7_test_src/7_test_2.nes",
        ScenarioLeaf::check_screen(50, 0x19ED_EC01_9A25_B69E)
    );
}
