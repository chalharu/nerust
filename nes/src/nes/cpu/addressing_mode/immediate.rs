// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use super::*;

pub(crate) struct Immediate;
impl AddressingMode for Immediate {
    fn next_func(
        &self,
        code: usize,
        register: &mut Register,
        opcodes: &mut Opcodes,
    ) -> Box<dyn CpuStepState> {
        let pc = register.get_pc() as usize;
        opcodes.get(code).next_func(pc, register)
    }

    fn name(&self) -> &'static str {
        "Immediate"
    }
}
