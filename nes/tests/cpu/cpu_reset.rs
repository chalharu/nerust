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
        ScenarioLeaf::check_screen(155, 0x440354DCC93B0821),
        ScenarioLeaf::reset(156),
        ScenarioLeaf::check_screen(255, 0xD3422C94B83715E9)
    );
}

#[test]
fn registers() {
    test!(
        "cpu/cpu_reset/registers.nes",
        ScenarioLeaf::check_screen(155, 0x440354DCC93B0821),
        ScenarioLeaf::reset(156),
        ScenarioLeaf::check_screen(255, 0x71F4ECB5DA8686D2)
    );
}
