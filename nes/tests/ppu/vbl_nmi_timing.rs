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
        ScenarioLeaf::check_screen(198, 0x65495211B3E6134A)
    );
}

#[test]
fn _02_vbl_timing() {
    test!(
        "ppu/vbl_nmi_timing/2.vbl_timing.nes",
        ScenarioLeaf::check_screen(179, 0x6E08E410EC698FD8)
    );
}

#[test]
fn _03_even_odd_frames() {
    test!(
        "ppu/vbl_nmi_timing/3.even_odd_frames.nes",
        ScenarioLeaf::check_screen(124, 0xC596360072486B12)
    );
}

#[test]
fn _04_vbl_clear_timing() {
    test!(
        "ppu/vbl_nmi_timing/4.vbl_clear_timing.nes",
        ScenarioLeaf::check_screen(140, 0xADE02067DC032C85)
    );
}

#[test]
fn _05_nmi_suppression() {
    test!(
        "ppu/vbl_nmi_timing/5.nmi_suppression.nes",
        ScenarioLeaf::check_screen(187, 0xDEE4849205CAA7D2)
    );
}

#[test]
fn _06_nmi_disable() {
    test!(
        "ppu/vbl_nmi_timing/6.nmi_disable.nes",
        ScenarioLeaf::check_screen(133, 0x731636C6A600A467)
    );
}

#[test]
fn _07_nmi_timing() {
    test!(
        "ppu/vbl_nmi_timing/7.nmi_timing.nes",
        ScenarioLeaf::check_screen(140, 0xC1BB1AAB8396D613)
    );
}
