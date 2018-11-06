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
        ScenarioLeaf::check_screen(344, 0xE00704F6A0376CBE)
    );
}

#[test]
fn _01_abs_x_wrap() {
    test!(
        "cpu/instr_misc/rom_singles/01-abs_x_wrap.nes",
        ScenarioLeaf::check_screen(15, 0x70C16D43E3AB469F)
    );
}

#[test]
fn _02_branch_wrap() {
    test!(
        "cpu/instr_misc/rom_singles/02-branch_wrap.nes",
        ScenarioLeaf::check_screen(20, 0x0BF2CADC2FD357FB)
    );
}

#[test]
fn _03_dummy_reads() {
    test!(
        "cpu/instr_misc/rom_singles/03-dummy_reads.nes",
        ScenarioLeaf::check_screen(70, 0xC92F20641AC33CF6)
    );
}

#[test]
fn _04_dummy_reads_apu() {
    test!(
        "cpu/instr_misc/rom_singles/04-dummy_reads_apu.nes",
        ScenarioLeaf::check_screen(165, 0xE3181E996F5F1FD0)
    );
}
