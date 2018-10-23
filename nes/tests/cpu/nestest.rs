// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use super::*;

#[test]
fn nestest() {
    test!(
        "nestest.nes",
        ScenarioLeaf::check_screen(15, 0x43073DD69063B0D2),
        ScenarioLeaf::standard_controller(15, Pad1(START), Pressed),
        ScenarioLeaf::standard_controller(16, Pad1(START), Released),
        ScenarioLeaf::check_screen(70, 0x01A4B722289CD31E),
        ScenarioLeaf::standard_controller(70, Pad1(SELECT), Pressed),
        ScenarioLeaf::standard_controller(71, Pad1(SELECT), Released),
        ScenarioLeaf::check_screen(75, 0xA5763C5F44A6FBED),
        ScenarioLeaf::standard_controller(75, Pad1(START), Pressed),
        ScenarioLeaf::standard_controller(76, Pad1(START), Released),
        ScenarioLeaf::check_screen(90, 0x6FBB66DD65D28A99)
    );
}
