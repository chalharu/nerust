// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use super::*;

#[test]
fn ppu_sprite_overflow() {
    test!(
        "ppu/ppu_sprite_overflow/ppu_sprite_overflow.nes",
        ScenarioLeaf::check_screen(480, 0x9026_DAD6_5555_ECA0)
    );
}
