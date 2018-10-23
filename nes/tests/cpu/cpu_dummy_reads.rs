// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use super::*;

#[test]
fn cpu_dummy_reads() {
    test!(
        "cpu_dummy_reads.nes",
        ScenarioLeaf::check_screen(50, 0x68A285C0C944073D)
    );
}
