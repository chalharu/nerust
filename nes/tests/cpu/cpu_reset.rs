// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use super::*;

#[test]
fn ram_after_reset() {
    test!(
        "cpu_reset/ram_after_reset.nes",
        ScenarioLeaf::check_screen(155, 0x6C18F33A360A267A),
        ScenarioLeaf::reset(156),
        ScenarioLeaf::check_screen(255, 0xA70256FE525B5712)
    );
}

#[test]
fn registers() {
    test!(
        "cpu_reset/registers.nes",
        ScenarioLeaf::check_screen(155, 0x6C18F33A360A267A),
        ScenarioLeaf::reset(156),
        ScenarioLeaf::check_screen(255, 0x15A2A5B1C285B8CE)
    );
}
