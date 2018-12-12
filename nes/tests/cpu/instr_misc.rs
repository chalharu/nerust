// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use super::*;

#[test]
fn instr_misc() {
    test!(
        "cpu/instr_misc/instr_misc.nes",
        ScenarioLeaf::check_screen(580, 0x20E234D5DB551AA9)
    );
}

#[test]
fn _01_abs_x_wrap() {
    test!(
        "cpu/instr_misc/rom_singles/01-abs_x_wrap.nes",
        ScenarioLeaf::check_screen(15, 0xE2900046F45BB66A)
    );
}

#[test]
fn _02_branch_wrap() {
    test!(
        "cpu/instr_misc/rom_singles/02-branch_wrap.nes",
        ScenarioLeaf::check_screen(20, 0x0341BD5B2530B417)
    );
}

#[test]
fn _03_dummy_reads() {
    test!(
        "cpu/instr_misc/rom_singles/03-dummy_reads.nes",
        ScenarioLeaf::check_screen(70, 0x0584AAAEB26972DC)
    );
}

#[test]
fn _04_dummy_reads_apu() {
    test!(
        "cpu/instr_misc/rom_singles/04-dummy_reads_apu.nes",
        ScenarioLeaf::check_screen(165, 0xAE30A6A220EF1A20)
    );
}
