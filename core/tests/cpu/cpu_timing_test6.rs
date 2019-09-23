// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use super::*;

#[test]
fn cpu_timing_test6() {
    run_test!(
        "cpu/cpu_timing_test6/cpu_timing_test.nes",
        ScenarioLeaf::check_screen(639, 0x172C_687E_69C0_6327)
    );
}
