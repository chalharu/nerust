// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use super::*;

#[test]
fn instr_timing() {
    test!(
        "instr_timing/instr_timing.nes",
        ScenarioLeaf::check_screen(1330, 0x5E0E057574FF467B)
    );
}
