// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use super::*;

#[test]
fn _1_basics() {
    run_test!(
        "ppu/sprite_overflow_tests/1.Basics.nes",
        ScenarioLeaf::check_screen(36, 0x50A5_A002_EEA2_256E)
    );
}

#[test]
fn _2_details() {
    run_test!(
        "ppu/sprite_overflow_tests/2.Details.nes",
        ScenarioLeaf::check_screen(36, 0xFAFB_3F12_716B_D507)
    );
}

#[test]
fn _3_timing() {
    run_test!(
        "ppu/sprite_overflow_tests/3.Timing.nes",
        ScenarioLeaf::check_screen(130, 0xF324_957B_1A6A_158E)
    );
}

#[test]
fn _4_obscure() {
    run_test!(
        "ppu/sprite_overflow_tests/4.Obscure.nes",
        ScenarioLeaf::check_screen(36, 0xDF35_5FCA_EA6D_930E)
    );
}

#[test]
fn _5_emulator() {
    run_test!(
        "ppu/sprite_overflow_tests/5.Emulator.nes",
        ScenarioLeaf::check_screen(36, 0x3FC6_8EEE_283C_E2C3)
    );
}
