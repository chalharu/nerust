// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use super::*;

#[test]
fn palette_ram() {
    test!(
        "blargg_ppu_tests_2005.09.15b/palette_ram.nes",
        ScenarioLeaf::check_screen(30, 0x85459C9BE19FB8A0)
    );
}

#[test]
fn power_up_palette() {
    test!(
        "blargg_ppu_tests_2005.09.15b/power_up_palette.nes",
        ScenarioLeaf::check_screen(30, 0x85459C9BE19FB8A0)
    );
}

#[test]
fn sprite_ram() {
    test!(
        "blargg_ppu_tests_2005.09.15b/sprite_ram.nes",
        ScenarioLeaf::check_screen(30, 0x85459C9BE19FB8A0)
    );
}

#[test]
fn vbl_clear_time() {
    test!(
        "blargg_ppu_tests_2005.09.15b/vbl_clear_time.nes",
        ScenarioLeaf::check_screen(30, 0x85459C9BE19FB8A0)
    );
}

#[test]
fn vram_access() {
    test!(
        "blargg_ppu_tests_2005.09.15b/vram_access.nes",
        ScenarioLeaf::check_screen(30, 0x85459C9BE19FB8A0)
    );
}
