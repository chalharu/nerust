// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use super::*;

#[test]
fn instr_test_v5() {
    test!(
        "cpu/instr_test-v5/all_instrs.nes",
        ScenarioLeaf::check_screen(2450, 0x4E02_09B2_3206_9DEA)
    );
}
