// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use super::*;

#[test]
fn ppu_sprite_hit() {
    run_test!(
        "ppu/ppu_sprite_hit/ppu_sprite_hit.nes",
        ScenarioLeaf::check_screen(800, 0xEB57_E169_78E4_5540)
    );
}

#[test]
fn _01_basics() {
    run_test!(
        "ppu/ppu_sprite_hit/rom_singles/01-basics.nes",
        ScenarioLeaf::check_screen(60, 0x10C1_27D0_9E7F_0585)
    );
}

#[test]
fn _02_alignment() {
    run_test!(
        "ppu/ppu_sprite_hit/rom_singles/02-alignment.nes",
        ScenarioLeaf::check_screen(85, 0xB288_39BA_DFF7_5598)
    );
}

#[test]
fn _03_corners() {
    run_test!(
        "ppu/ppu_sprite_hit/rom_singles/03-corners.nes",
        ScenarioLeaf::check_screen(55, 0xA01E_C12C_BB5B_AA86)
    );
}

#[test]
fn _04_flip() {
    run_test!(
        "ppu/ppu_sprite_hit/rom_singles/04-flip.nes",
        ScenarioLeaf::check_screen(40, 0x2748_0614_E261_6F04)
    );
}

#[test]
fn _05_left_clip() {
    run_test!(
        "ppu/ppu_sprite_hit/rom_singles/05-left_clip.nes",
        ScenarioLeaf::check_screen(52, 0xDAF2_913B_574D_EFED)
    );
}

#[test]
fn _06_right_edge() {
    run_test!(
        "ppu/ppu_sprite_hit/rom_singles/06-right_edge.nes",
        ScenarioLeaf::check_screen(45, 0xB453_214D_284B_A5DE)
    );
}

#[test]
fn _07_screen_bottom() {
    run_test!(
        "ppu/ppu_sprite_hit/rom_singles/07-screen_bottom.nes",
        ScenarioLeaf::check_screen(50, 0xC6F8_94DB_61A5_E7C4)
    );
}

#[test]
fn _08_double_height() {
    run_test!(
        "ppu/ppu_sprite_hit/rom_singles/08-double_height.nes",
        ScenarioLeaf::check_screen(40, 0x7336_D45B_93EF_0C23)
    );
}

#[test]
fn _09_timing() {
    run_test!(
        "ppu/ppu_sprite_hit/rom_singles/09-timing.nes",
        ScenarioLeaf::check_screen(200, 0xC3F2_0913_7CAA_75E2)
    );
}

#[test]
fn _10_timing_order() {
    run_test!(
        "ppu/ppu_sprite_hit/rom_singles/10-timing_order.nes",
        ScenarioLeaf::check_screen(90, 0x7B94_8A10_ACE7_0AD0)
    );
}
