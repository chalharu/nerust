// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use super::*;

#[test]
fn ram_after_reset() {
    test!(
        "cpu/cpu_reset/ram_after_reset.nes",
        ScenarioLeaf::check_screen(155, 0x4403_54DC_C93B_0821),
        ScenarioLeaf::reset(156),
        ScenarioLeaf::check_screen(255, 0xD342_2C94_B837_15E9)
    );
}

#[test]
fn registers() {
    test!(
        "cpu/cpu_reset/registers.nes",
        ScenarioLeaf::check_screen(155, 0x4403_54DC_C93B_0821),
        ScenarioLeaf::reset(156),
        ScenarioLeaf::check_screen(255, 0x71F4_ECB5_DA86_86D2)
    );
}
