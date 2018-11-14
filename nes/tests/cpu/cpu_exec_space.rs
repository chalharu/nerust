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
        ScenarioLeaf::check_screen(45, 0xFA8F4E7F0ECDD92F)
    );
}

#[test]
fn test_cpu_exec_space_apu() {
    test!(
        "cpu/cpu_exec_space/test_cpu_exec_space_apu.nes",
        ScenarioLeaf::check_screen(295, 0x3A0C2ED9AA73D9F4)
    );
}
