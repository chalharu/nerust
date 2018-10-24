// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use super::*;

#[test]
fn cpu_interrupts() {
    test!(
        "cpu_interrupts_v2/cpu_interrupts.nes",
        ScenarioLeaf::check_screen(45, 0xB1866B91E4771BAB)
    );
    panic!("Not implemented");
}
