// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use super::*;

#[test]
fn _2_test_0() {
    run_test!(
        "mapper/2_test_src/2_test_0.nes",
        ScenarioLeaf::check_screen(50, 0x01C3_BF21_8899_DD55)
    );
}

#[test]
fn _2_test_1() {
    run_test!(
        "mapper/2_test_src/2_test_1.nes",
        ScenarioLeaf::check_screen(50, 0x01C3_BF21_8899_DD55)
    );
}

#[test]
fn _2_test_2() {
    run_test!(
        "mapper/2_test_src/2_test_2.nes",
        ScenarioLeaf::check_screen(50, 0x7D49_67ED_8CD2_64A0)
    );
}
