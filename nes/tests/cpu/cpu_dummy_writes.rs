// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use super::*;

#[test]
fn cpu_dummy_writes_oam() {
    test!(
        "cpu_dummy_writes/cpu_dummy_writes_oam.nes",
        ScenarioLeaf::check_screen(330, 0x6AB7DBF3764D9D43)
    );
}

#[test]
fn cpu_dummy_writes_ppumem() {
    test!(
        "cpu_dummy_writes/cpu_dummy_writes_ppumem.nes",
        ScenarioLeaf::check_screen(240, 0xF8A9BE71A106B451)
    );
}
