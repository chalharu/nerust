// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use super::*;

#[test]
fn _01_basics() {
    test!(
        "ppu/sprite_hit_tests_2005.10.05/01.basics.nes",
        ScenarioLeaf::check_screen(36, 0x89392E806F5682F4)
    );
}

#[test]
fn _02_alignment() {
    test!(
        "ppu/sprite_hit_tests_2005.10.05/02.alignment.nes",
        ScenarioLeaf::check_screen(34, 0x75D8550D59B6F72B)
    );
}

#[test]
fn _03_corners() {
    test!(
        "ppu/sprite_hit_tests_2005.10.05/03.corners.nes",
        ScenarioLeaf::check_screen(34, 0x2983264967F6A253)
    );
}

#[test]
fn _04_flip() {
    test!(
        "ppu/sprite_hit_tests_2005.10.05/04.flip.nes",
        ScenarioLeaf::check_screen(34, 0x9BAF184F5F15E8A7)
    );
}

#[test]
fn _05_left_clip() {
    test!(
        "ppu/sprite_hit_tests_2005.10.05/05.left_clip.nes",
        ScenarioLeaf::check_screen(34, 0x14DE22738C3636C0)
    );
}

#[test]
fn _06_right_edge() {
    test!(
        "ppu/sprite_hit_tests_2005.10.05/06.right_edge.nes",
        ScenarioLeaf::check_screen(34, 0x2270DD899C0E1480)
    );
}

#[test]
fn _07_screen_bottom() {
    test!(
        "ppu/sprite_hit_tests_2005.10.05/07.screen_bottom.nes",
        ScenarioLeaf::check_screen(34, 0x5571EB62B8928090)
    );
}

#[test]
fn _08_double_height() {
    test!(
        "ppu/sprite_hit_tests_2005.10.05/08.double_height.nes",
        ScenarioLeaf::check_screen(34, 0xC5EE8DB0ABBD48ED)
    );
}

#[test]
fn _09_timing_basics() {
    test!(
        "ppu/sprite_hit_tests_2005.10.05/09.timing_basics.nes",
        ScenarioLeaf::check_screen(80, 0x8CED0595749BE2DA)
    );
}

#[test]
fn _10_timing_order() {
    test!(
        "ppu/sprite_hit_tests_2005.10.05/10.timing_order.nes",
        ScenarioLeaf::check_screen(60, 0xBDE510E7036C02DD)
    );
}

#[test]
fn _11_edge_timing() {
    test!(
        "ppu/sprite_hit_tests_2005.10.05/11.edge_timing.nes",
        ScenarioLeaf::check_screen(80, 0xB3C59FBA25A122C8)
    );
}
