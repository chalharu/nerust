// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use super::*;

#[test]
fn test_cpu_flag_concurrency() {
    test!(
        "cpu/cpu_flag_concurrency/test_cpu_flag_concurrency.nes",
        // ScenarioLeaf::check_screen(850, 0xF08167007525306C)
        ScenarioLeaf::check_screen(850, 0x65CC7F8C6F7B5C41)
    );
}
