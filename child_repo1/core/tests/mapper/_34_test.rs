// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use super::*;

#[test]
fn _34_test_1() {
    test!(
        "mapper/34_test_src/34_test_1.nes",
        ScenarioLeaf::check_screen(70, 0x4656_955C_2419_76B9)
    );
}

#[test]
fn _34_test_2() {
    test!(
        "mapper/34_test_src/34_test_2.nes",
        ScenarioLeaf::check_screen(70, 0x00C9_497C_3EC4_44FF)
    );
}
