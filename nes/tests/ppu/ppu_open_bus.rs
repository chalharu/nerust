// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use super::*;

#[test]
fn ppu_open_bus() {
    test!(
        "ppu/ppu_open_bus/ppu_open_bus.nes",
        ScenarioLeaf::check_screen(30, 0x04C8F601831BF986)
    );
    panic!("Not implemented");
}
