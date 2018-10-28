// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use super::*;

#[test]
fn test_9() {
    test!(
        "apu/test_apu_m/test_9.nes",
        ScenarioLeaf::check_screen(50, 0x4E18EA8AE506BFC5)
    );
}

#[test]
fn test_10() {
    test!(
        "apu/test_apu_m/test_10.nes",
        ScenarioLeaf::check_screen(50, 0x4E18EA8AE506BFC5)
    );
}

#[test]
fn test_11() {
    test!(
        "apu/test_apu_m/test_11.nes",
        ScenarioLeaf::check_screen(50, 0x4E18EA8AE506BFC5)
    );
}
