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
        ScenarioLeaf::check_screen(36, 0xB6B8D4F4C83C2F3A)
    );
}

#[test]
fn _02_alignment() {
    test!(
        "ppu/sprite_hit_tests_2005.10.05/02.alignment.nes",
        ScenarioLeaf::check_screen(34, 0xDED8BAE9675ECFFC)
    );
}

#[test]
fn _03_corners() {
    test!(
        "ppu/sprite_hit_tests_2005.10.05/03.corners.nes",
        ScenarioLeaf::check_screen(34, 0x299F099E767A7FBC)
    );
}

#[test]
fn _04_flip() {
    test!(
        "ppu/sprite_hit_tests_2005.10.05/04.flip.nes",
        ScenarioLeaf::check_screen(34, 0xEA13722405663737)
    );
}

#[test]
fn _05_left_clip() {
    test!(
        "ppu/sprite_hit_tests_2005.10.05/05.left_clip.nes",
        ScenarioLeaf::check_screen(34, 0xCA3F0DFC6A85445F)
    );
}

#[test]
fn _06_right_edge() {
    test!(
        "ppu/sprite_hit_tests_2005.10.05/06.right_edge.nes",
        ScenarioLeaf::check_screen(34, 0x36BE2A768F7651FB)
    );
}

#[test]
fn _07_screen_bottom() {
    test!(
        "ppu/sprite_hit_tests_2005.10.05/07.screen_bottom.nes",
        ScenarioLeaf::check_screen(34, 0xF5612439038423A9)
    );
}

#[test]
fn _08_double_height() {
    test!(
        "ppu/sprite_hit_tests_2005.10.05/08.double_height.nes",
        ScenarioLeaf::check_screen(34, 0xD124539B482668D5)
    );
}

#[test]
fn _09_timing_basics() {
    test!(
        "ppu/sprite_hit_tests_2005.10.05/09.timing_basics.nes",
        ScenarioLeaf::check_screen(80, 0xB71D1F7A8C2BED67)
    );
}

#[test]
fn _10_timing_order() {
    test!(
        "ppu/sprite_hit_tests_2005.10.05/10.timing_order.nes",
        ScenarioLeaf::check_screen(60, 0x35700A9026341B07)
    );
}

#[test]
fn _11_edge_timing() {
    test!(
        "ppu/sprite_hit_tests_2005.10.05/11.edge_timing.nes",
        ScenarioLeaf::check_screen(80, 0x5ECA913BB9982F14)
    );
}
