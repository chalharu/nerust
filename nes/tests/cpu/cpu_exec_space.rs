// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use super::*;

#[test]
fn test_cpu_exec_space_ppuio() {
    test!(
        "cpu/cpu_exec_space/test_cpu_exec_space_ppuio.nes",
        ScenarioLeaf::check_screen(45, 0xFA8F_4E7F_0ECD_D92F)
    );
}

#[test]
fn test_cpu_exec_space_apu() {
    test!(
        "cpu/cpu_exec_space/test_cpu_exec_space_apu.nes",
        ScenarioLeaf::check_screen(295, 0x3A0C_2ED9_AA73_D9F4)
    );
}
