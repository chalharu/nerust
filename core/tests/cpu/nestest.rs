// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use super::*;

#[test]
fn nestest() {
    run_test!(
        "cpu/nestest.nes",
        ScenarioLeaf::check_screen(15, 0x4640_33EF_DAB1_1D8E),
        ScenarioLeaf::standard_controller(15, Pad1(START), Pressed),
        ScenarioLeaf::standard_controller(16, Pad1(START), Released),
        ScenarioLeaf::check_screen(70, 0xBE54_DF8C_F9FB_E026),
        ScenarioLeaf::standard_controller(70, Pad1(SELECT), Pressed),
        ScenarioLeaf::standard_controller(71, Pad1(SELECT), Released),
        ScenarioLeaf::check_screen(75, 0x9D08_2986_B6F8_DF51),
        ScenarioLeaf::standard_controller(75, Pad1(START), Pressed),
        ScenarioLeaf::standard_controller(76, Pad1(START), Released),
        ScenarioLeaf::check_screen(90, 0xBACF_3F4F_CBF5_718C)
    );
}
