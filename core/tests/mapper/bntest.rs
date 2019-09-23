// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use super::*;

#[test]
fn bntest_aorom() {
    run_test!(
        "mapper/bntest/bntest_aorom.nes",
        ScenarioLeaf::check_screen(15, 0xAD9D_23DD_8E57_3B19)
    );
}

#[test]
fn bntest_h() {
    run_test!(
        "mapper/bntest/bntest_h.nes",
        ScenarioLeaf::check_screen(15, 0x8108_A6D2_A9D9_C28A)
    );
}

#[test]
fn bntest_v() {
    run_test!(
        "mapper/bntest/bntest_v.nes",
        ScenarioLeaf::check_screen(15, 0x4E34_969E_C01E_A621)
    );
}
