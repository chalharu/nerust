// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use super::*;

#[test]
fn nestest() {
    test!(
        "cpu/nestest.nes",
        ScenarioLeaf::check_screen(15, 0x464033EFDAB11D8E),
        ScenarioLeaf::standard_controller(15, Pad1(START), Pressed),
        ScenarioLeaf::standard_controller(16, Pad1(START), Released),
        ScenarioLeaf::check_screen(70, 0xBE54DF8CF9FBE026),
        ScenarioLeaf::standard_controller(70, Pad1(SELECT), Pressed),
        ScenarioLeaf::standard_controller(71, Pad1(SELECT), Released),
        ScenarioLeaf::check_screen(75, 0x9D082986B6F8DF51),
        ScenarioLeaf::standard_controller(75, Pad1(START), Pressed),
        ScenarioLeaf::standard_controller(76, Pad1(START), Released),
        ScenarioLeaf::check_screen(90, 0xBACF3F4FCBF5718C)
    );
}
