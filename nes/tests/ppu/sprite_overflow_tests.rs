// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use super::*;

#[test]
fn _1_basics() {
    test!(
        "ppu/sprite_overflow_tests/1.Basics.nes",
        ScenarioLeaf::check_screen(36, 0x50A5A002EEA2256E)
    );
}

#[test]
fn _2_details() {
    test!(
        "ppu/sprite_overflow_tests/2.Details.nes",
        ScenarioLeaf::check_screen(36, 0xFAFB3F12716BD507)
    );
}

#[test]
fn _3_timing() {
    test!(
        "ppu/sprite_overflow_tests/3.Timing.nes",
        ScenarioLeaf::check_screen(130, 0xF324957B1A6A158E)
    );
}

#[test]
fn _4_obscure() {
    test!(
        "ppu/sprite_overflow_tests/4.Obscure.nes",
        ScenarioLeaf::check_screen(36, 0xDF355FCAEA6D930E)
    );
}

#[test]
fn _5_emulator() {
    test!(
        "ppu/sprite_overflow_tests/5.Emulator.nes",
        ScenarioLeaf::check_screen(36, 0x3FC68EEE283CE2C3)
    );
}
