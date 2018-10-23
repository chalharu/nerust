// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use super::*;

#[test]
fn test_cpu_exec_space_ppuio() {
    test!(
        "cpu_exec_space/test_cpu_exec_space_ppuio.nes",
        ScenarioLeaf::check_screen(45, 0xB1866B91E4771BAB)
    );
}

#[test]
fn test_cpu_exec_space_apu() {
    test!(
        "cpu_exec_space/test_cpu_exec_space_apu.nes",
        ScenarioLeaf::check_screen(295, 0x28EE2FAC59284B74)
    );
}
