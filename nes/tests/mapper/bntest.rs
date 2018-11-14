// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use super::*;

#[test]
fn bntest_aorom() {
    test!(
        "mapper/bntest/bntest_aorom.nes",
        ScenarioLeaf::check_screen(639, 0x172C687E69C06327)
    );
    panic!("Not implemented");
}

#[test]
fn bntest_h() {
    test!(
        "mapper/bntest/bntest_h.nes",
        ScenarioLeaf::check_screen(639, 0x172C687E69C06327)
    );
    panic!("Not implemented");
}

#[test]
fn bntest_v() {
    test!(
        "mapper/bntest/bntest_v.nes",
        ScenarioLeaf::check_screen(639, 0x172C687E69C06327)
    );
    panic!("Not implemented");
}
