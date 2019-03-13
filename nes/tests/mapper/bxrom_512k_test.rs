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
        ScenarioLeaf::check_screen(15, 0x56C4_FA7D_D5BF_D57B),
        ScenarioLeaf::check_screen(45, 0x187F_9846_2E73_7C08),
        ScenarioLeaf::check_screen(75, 0x4C8D_8ADD_B93D_2691),
        ScenarioLeaf::check_screen(105, 0xEAEC_D98D_0A62_96B1),
        ScenarioLeaf::check_screen(135, 0x9383_A55C_004D_8DEF),
        ScenarioLeaf::check_screen(165, 0x7D1C_E0BB_BEBC_8565),
        ScenarioLeaf::check_screen(195, 0x14B6_7E41_3E85_D3E7),
        ScenarioLeaf::check_screen(225, 0x7C09_32B4_22A4_63CD),
        ScenarioLeaf::check_screen(255, 0x3B34_1C14_BBCB_4637),
        ScenarioLeaf::check_screen(285, 0x9815_7B65_13DB_1D92),
        ScenarioLeaf::check_screen(315, 0xD4A1_35BE_EFDE_569A),
        ScenarioLeaf::check_screen(345, 0x7615_A5B0_91DB_6D45),
        ScenarioLeaf::check_screen(375, 0xB362_A38A_4EF2_5CD1),
        ScenarioLeaf::check_screen(405, 0xE9C9_3AA3_BC26_9AA4),
        ScenarioLeaf::check_screen(435, 0xA4E4_6F56_9145_7ADE),
        ScenarioLeaf::check_screen(465, 0x3B05_8504_58FB_A8FF),
        ScenarioLeaf::check_screen(495, 0x56C4_FA7D_D5BF_D57B)
    );
}
