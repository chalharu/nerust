// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use super::*;

#[test]
fn cpu_dummy_reads() {
    run_test!(
        "cpu/cpu_dummy_reads.nes",
        ScenarioLeaf::check_screen(50, 0x1384_1FED_B44D_C75D)
    );
}
