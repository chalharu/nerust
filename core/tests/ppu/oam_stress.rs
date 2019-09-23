// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use super::*;

#[test]
fn oam_stress() {
    run_test!(
        "ppu/oam_stress/oam_stress.nes",
        ScenarioLeaf::check_screen(1740, 0x8657_96B2_97C3_0C6B)
    );
}
