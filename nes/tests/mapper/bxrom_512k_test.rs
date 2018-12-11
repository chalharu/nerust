// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use super::*;

#[test]
fn bxrom_512k_test() {
    test!(
        "mapper/bxrom_512k_test.nes",
        ScenarioLeaf::check_screen(15, 0x56C4FA7DD5BFD57B),
        ScenarioLeaf::check_screen(45, 0x187F98462E737C08),
        ScenarioLeaf::check_screen(75, 0x4C8D8ADDB93D2691),
        ScenarioLeaf::check_screen(105, 0xEAECD98D0A6296B1),
        ScenarioLeaf::check_screen(135, 0x9383A55C004D8DEF),
        ScenarioLeaf::check_screen(165, 0x7D1CE0BBBEBC8565),
        ScenarioLeaf::check_screen(195, 0x14B67E413E85D3E7),
        ScenarioLeaf::check_screen(225, 0x7C0932B422A463CD),
        ScenarioLeaf::check_screen(255, 0x3B341C14BBCB4637),
        ScenarioLeaf::check_screen(285, 0x98157B6513DB1D92),
        ScenarioLeaf::check_screen(315, 0xD4A135BEEFDE569A),
        ScenarioLeaf::check_screen(345, 0x7615A5B091DB6D45),
        ScenarioLeaf::check_screen(375, 0xB362A38A4EF25CD1),
        ScenarioLeaf::check_screen(405, 0xE9C93AA3BC269AA4),
        ScenarioLeaf::check_screen(435, 0xA4E46F5691457ADE),
        ScenarioLeaf::check_screen(465, 0x3B05850458FBA8FF),
        ScenarioLeaf::check_screen(495, 0x56C4FA7DD5BFD57B)
    );
}
