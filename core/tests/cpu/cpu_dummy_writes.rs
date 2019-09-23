// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use super::*;

#[test]
fn cpu_dummy_writes_oam() {
    run_test!(
        "cpu/cpu_dummy_writes/cpu_dummy_writes_oam.nes",
        ScenarioLeaf::check_screen(330, 0xF25F_B885_BF1F_8DE2)
    );
}

#[test]
fn cpu_dummy_writes_ppumem() {
    run_test!(
        "cpu/cpu_dummy_writes/cpu_dummy_writes_ppumem.nes",
        ScenarioLeaf::check_screen(240, 0x6840_4497_D176_CD2A)
    );
}
