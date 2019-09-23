// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use super::*;

#[test]
fn _01_basics() {
    run_test!(
        "ppu/sprite_hit_tests_2005.10.05/01.basics.nes",
        ScenarioLeaf::check_screen(36, 0xB6B8_D4F4_C83C_2F3A)
    );
}

#[test]
fn _02_alignment() {
    run_test!(
        "ppu/sprite_hit_tests_2005.10.05/02.alignment.nes",
        ScenarioLeaf::check_screen(34, 0xDED8_BAE9_675E_CFFC)
    );
}

#[test]
fn _03_corners() {
    run_test!(
        "ppu/sprite_hit_tests_2005.10.05/03.corners.nes",
        ScenarioLeaf::check_screen(34, 0x299F_099E_767A_7FBC)
    );
}

#[test]
fn _04_flip() {
    run_test!(
        "ppu/sprite_hit_tests_2005.10.05/04.flip.nes",
        ScenarioLeaf::check_screen(34, 0xEA13_7224_0566_3737)
    );
}

#[test]
fn _05_left_clip() {
    run_test!(
        "ppu/sprite_hit_tests_2005.10.05/05.left_clip.nes",
        ScenarioLeaf::check_screen(34, 0xCA3F_0DFC_6A85_445F)
    );
}

#[test]
fn _06_right_edge() {
    run_test!(
        "ppu/sprite_hit_tests_2005.10.05/06.right_edge.nes",
        ScenarioLeaf::check_screen(34, 0x36BE_2A76_8F76_51FB)
    );
}

#[test]
fn _07_screen_bottom() {
    run_test!(
        "ppu/sprite_hit_tests_2005.10.05/07.screen_bottom.nes",
        ScenarioLeaf::check_screen(34, 0xF561_2439_0384_23A9)
    );
}

#[test]
fn _08_double_height() {
    run_test!(
        "ppu/sprite_hit_tests_2005.10.05/08.double_height.nes",
        ScenarioLeaf::check_screen(34, 0xD124_539B_4826_68D5)
    );
}

#[test]
fn _09_timing_basics() {
    run_test!(
        "ppu/sprite_hit_tests_2005.10.05/09.timing_basics.nes",
        ScenarioLeaf::check_screen(80, 0xB71D_1F7A_8C2B_ED67)
    );
}

#[test]
fn _10_timing_order() {
    run_test!(
        "ppu/sprite_hit_tests_2005.10.05/10.timing_order.nes",
        ScenarioLeaf::check_screen(60, 0x3570_0A90_2634_1B07)
    );
}

#[test]
fn _11_edge_timing() {
    run_test!(
        "ppu/sprite_hit_tests_2005.10.05/11.edge_timing.nes",
        ScenarioLeaf::check_screen(80, 0x5ECA_913B_B998_2F14)
    );
}
