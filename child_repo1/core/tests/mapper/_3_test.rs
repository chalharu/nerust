// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use super::*;

#[test]
fn _3_test_0() {
    // TODO: テストROMに　PRG RAMの表示が必ずYESとなるバグがあるように見受けられる
    test!(
        "mapper/3_test_src/3_test_0.nes",
        ScenarioLeaf::check_screen(50, 0xE3E5_E7C7_A502_A3DD)
    );
}

#[test]
fn _3_test_1() {
    // TODO: テストROMに　PRG RAMの表示が必ずYESとなるバグがあるように見受けられる
    test!(
        "mapper/3_test_src/3_test_1.nes",
        ScenarioLeaf::check_screen(50, 0xE3E5_E7C7_A502_A3DD)
    );
}

#[test]
fn _3_test_2() {
    // TODO: テストROMに　PRG RAMの表示が必ずYESとなるバグがあるように見受けられる
    test!(
        "mapper/3_test_src/3_test_2.nes",
        ScenarioLeaf::check_screen(50, 0x14F4_F5CC_EB46_665A)
    );
}
