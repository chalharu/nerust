// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use super::*;

fn increment(register: &mut Register, data: u8) -> u8 {
    let result = data.wrapping_add(1);
    register.set_nz_from_value(result);
    result
}

accumulate!(Inx, Register::get_x, Register::set_x, increment);
accumulate!(Iny, Register::get_y, Register::set_y, increment);
accumulate_memory!(Inc, increment);
