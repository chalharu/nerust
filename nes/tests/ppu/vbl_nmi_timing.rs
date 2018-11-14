// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use super::*;

#[test]
fn _01_frame_basics() {
    test!(
        "ppu/vbl_nmi_timing/1.frame_basics.nes",
        ScenarioLeaf::check_screen(198, 0x1731EB1B8C091E47)
    );
}

#[test]
fn _02_vbl_timing() {
    test!(
        "ppu/vbl_nmi_timing/2.vbl_timing.nes",
        ScenarioLeaf::check_screen(179, 0xE51D5454497CBFDB)
    );
}

#[test]
fn _03_even_odd_frames() {
    test!(
        "ppu/vbl_nmi_timing/3.even_odd_frames.nes",
        ScenarioLeaf::check_screen(124, 0x537008D7E0C3FF45)
    );
}

#[test]
fn _04_vbl_clear_timing() {
    test!(
        "ppu/vbl_nmi_timing/4.vbl_clear_timing.nes",
        ScenarioLeaf::check_screen(140, 0x7E5D5B53E1C2703F)
    );
}

#[test]
fn _05_nmi_suppression() {
    test!(
        "ppu/vbl_nmi_timing/5.nmi_suppression.nes",
        ScenarioLeaf::check_screen(187, 0xAC7573AA9EE324C0)
    );
}

#[test]
fn _06_nmi_disable() {
    test!(
        "ppu/vbl_nmi_timing/6.nmi_disable.nes",
        ScenarioLeaf::check_screen(133, 0xC5509E44ABF95C7D)
    );
}

#[test]
fn _07_nmi_timing() {
    test!(
        "ppu/vbl_nmi_timing/7.nmi_timing.nes",
        ScenarioLeaf::check_screen(140, 0x47D4DAE682D8F8D0)
    );
}
