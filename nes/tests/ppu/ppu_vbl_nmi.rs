// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use super::*;

#[test]
fn ppu_vbl_nmi() {
    test!(
        "ppu/ppu_vbl_nmi/ppu_vbl_nmi.nes",
        ScenarioLeaf::check_screen(1640, 0xEB57_E169_78E4_5540)
    );
}
