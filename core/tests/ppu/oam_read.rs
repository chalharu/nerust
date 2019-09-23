// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use super::*;

#[test]
fn oam_read() {
    run_test!(
        "ppu/oam_read/oam_read.nes",
        ScenarioLeaf::check_screen(30, 0x09D0_3496_0D5B_F704)
    );
}
