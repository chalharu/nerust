// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use super::*;

#[test]
fn ppu_sprite_hit() {
    test!(
        "ppu/ppu_sprite_hit/ppu_sprite_hit.nes",
        ScenarioLeaf::check_screen(600, 0x1C836FC773555051)
    );
}

#[test]
fn _01_basics() {
    test!(
        "ppu/ppu_sprite_hit/rom_singles/01-basics.nes",
        ScenarioLeaf::check_screen(60, 0x42CFACAA9A15D013)
    );
}

#[test]
fn _02_alignment() {
    test!(
        "ppu/ppu_sprite_hit/rom_singles/02-alignment.nes",
        ScenarioLeaf::check_screen(85, 0xB1B5C43737FB16EF)
    );
}

#[test]
fn _03_corners() {
    test!(
        "ppu/ppu_sprite_hit/rom_singles/03-corners.nes",
        ScenarioLeaf::check_screen(55, 0x700017AB5E656ECD)
    );
}

#[test]
fn _04_flip() {
    test!(
        "ppu/ppu_sprite_hit/rom_singles/04-flip.nes",
        ScenarioLeaf::check_screen(40, 0x549F8BF6A80774B1)
    );
}

#[test]
fn _05_left_clip() {
    test!(
        "ppu/ppu_sprite_hit/rom_singles/05-left_clip.nes",
        ScenarioLeaf::check_screen(52, 0xCB232878F232040A)
    );
}

#[test]
fn _06_right_edge() {
    test!(
        "ppu/ppu_sprite_hit/rom_singles/06-right_edge.nes",
        ScenarioLeaf::check_screen(45, 0x89567E17B702EED4)
    );
}

#[test]
fn _07_screen_bottom() {
    test!(
        "ppu/ppu_sprite_hit/rom_singles/07-screen_bottom.nes",
        ScenarioLeaf::check_screen(50, 0xAF9129A6D7E48B2B)
    );
}

#[test]
fn _08_double_height() {
    test!(
        "ppu/ppu_sprite_hit/rom_singles/08-double_height.nes",
        ScenarioLeaf::check_screen(40, 0xCCFC308B39369365)
    );
}

#[test]
fn _09_timing() {
    test!(
        "ppu/ppu_sprite_hit/rom_singles/09-timing.nes",
        ScenarioLeaf::check_screen(200, 0x0EF3FB81DCF0DE18)
    );
}

#[test]
fn _10_timing_order() {
    test!(
        "ppu/ppu_sprite_hit/rom_singles/10-timing_order.nes",
        ScenarioLeaf::check_screen(90, 0x65F210E6178421E0)
    );
}
