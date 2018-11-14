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
        ScenarioLeaf::check_screen(600, 0xEB57E16978E45540)
    );
}

#[test]
fn _01_basics() {
    test!(
        "ppu/ppu_sprite_hit/rom_singles/01-basics.nes",
        ScenarioLeaf::check_screen(60, 0x10C127D09E7F0585)
    );
}

#[test]
fn _02_alignment() {
    test!(
        "ppu/ppu_sprite_hit/rom_singles/02-alignment.nes",
        ScenarioLeaf::check_screen(85, 0xB28839BADFF75598)
    );
}

#[test]
fn _03_corners() {
    test!(
        "ppu/ppu_sprite_hit/rom_singles/03-corners.nes",
        ScenarioLeaf::check_screen(55, 0xA01EC12CBB5BAA86)
    );
}

#[test]
fn _04_flip() {
    test!(
        "ppu/ppu_sprite_hit/rom_singles/04-flip.nes",
        ScenarioLeaf::check_screen(40, 0x27480614E2616F04)
    );
}

#[test]
fn _05_left_clip() {
    test!(
        "ppu/ppu_sprite_hit/rom_singles/05-left_clip.nes",
        ScenarioLeaf::check_screen(52, 0xDAF2913B574DEFED)
    );
}

#[test]
fn _06_right_edge() {
    test!(
        "ppu/ppu_sprite_hit/rom_singles/06-right_edge.nes",
        ScenarioLeaf::check_screen(45, 0xB453214D284BA5DE)
    );
}

#[test]
fn _07_screen_bottom() {
    test!(
        "ppu/ppu_sprite_hit/rom_singles/07-screen_bottom.nes",
        ScenarioLeaf::check_screen(50, 0xC6F894DB61A5E7C4)
    );
}

#[test]
fn _08_double_height() {
    test!(
        "ppu/ppu_sprite_hit/rom_singles/08-double_height.nes",
        ScenarioLeaf::check_screen(40, 0x7336D45B93EF0C23)
    );
}

#[test]
fn _09_timing() {
    test!(
        "ppu/ppu_sprite_hit/rom_singles/09-timing.nes",
        ScenarioLeaf::check_screen(200, 0xC3F209137CAA75E2)
    );
}

#[test]
fn _10_timing_order() {
    test!(
        "ppu/ppu_sprite_hit/rom_singles/10-timing_order.nes",
        ScenarioLeaf::check_screen(90, 0x7B948A10ACE70AD0)
    );
}
