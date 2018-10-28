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
        ScenarioLeaf::check_screen(36, 0x64673F9E8279B5DA)
    );
}

#[test]
fn _2_details() {
    test!(
        "ppu/sprite_overflow_tests/2.Details.nes",
        ScenarioLeaf::check_screen(36, 0x6857729005806691)
    );
}

#[test]
fn _3_timing() {
    test!(
        "ppu/sprite_overflow_tests/3.Timing.nes",
        ScenarioLeaf::check_screen(130, 0xBF60CA9E1BDCFA3B)
    );
}

#[test]
fn _4_obscure() {
    test!(
        "ppu/sprite_overflow_tests/4.Obscure.nes",
        ScenarioLeaf::check_screen(36, 0xE6B70C24953720D2)
    );
}

#[test]
fn _5_emulator() {
    test!(
        "ppu/sprite_overflow_tests/5.Emulator.nes",
        ScenarioLeaf::check_screen(36, 0x0F70D5EEDE382586)
    );
}
