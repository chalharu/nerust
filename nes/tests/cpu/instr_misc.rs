// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use super::*;

#[test]
fn instr_misc() {
    test!(
        "cpu/instr_misc/instr_misc.nes",
        ScenarioLeaf::check_screen(344, 0xE00704F6A0376CBE)
    );
}
