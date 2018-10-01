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
        interrupt: &mut Interrupt,
    ) -> Box<dyn CpuStepState> {
        let pc = register.get_pc();
        register.set_pc(pc.wrapping_add(1));
        opcodes
            .get(code)
            .next_func(pc as usize, register, interrupt)
    }

    fn name(&self) -> &'static str {
        "Immediate"
    }
}
