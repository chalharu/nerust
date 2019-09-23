// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use super::*;

#[test]
fn _01_frame_basics() {
    run_test!(
        "ppu/vbl_nmi_timing/1.frame_basics.nes",
        ScenarioLeaf::check_screen(198, 0x1731_EB1B_8C09_1E47)
    );
}

#[test]
fn _02_vbl_timing() {
    run_test!(
        "ppu/vbl_nmi_timing/2.vbl_timing.nes",
        ScenarioLeaf::check_screen(179, 0xE51D_5454_497C_BFDB)
    );
}

#[test]
fn _03_even_odd_frames() {
    run_test!(
        "ppu/vbl_nmi_timing/3.even_odd_frames.nes",
        ScenarioLeaf::check_screen(124, 0x5370_08D7_E0C3_FF45)
    );
}

#[test]
fn _04_vbl_clear_timing() {
    run_test!(
        "ppu/vbl_nmi_timing/4.vbl_clear_timing.nes",
        ScenarioLeaf::check_screen(140, 0x7E5D_5B53_E1C2_703F)
    );
}

#[test]
fn _05_nmi_suppression() {
    run_test!(
        "ppu/vbl_nmi_timing/5.nmi_suppression.nes",
        ScenarioLeaf::check_screen(187, 0xAC75_73AA_9EE3_24C0)
    );
}

#[test]
fn _06_nmi_disable() {
    run_test!(
        "ppu/vbl_nmi_timing/6.nmi_disable.nes",
        ScenarioLeaf::check_screen(133, 0xC550_9E44_ABF9_5C7D)
    );
}

#[test]
fn _07_nmi_timing() {
    run_test!(
        "ppu/vbl_nmi_timing/7.nmi_timing.nes",
        ScenarioLeaf::check_screen(140, 0x47D4_DAE6_82D8_F8D0)
    );
}
