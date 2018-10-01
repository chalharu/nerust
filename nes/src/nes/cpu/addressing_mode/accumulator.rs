// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use super::*;

pub(crate) struct Accumulator;
impl AddressingMode for Accumulator {
    fn next_func(
        &self,
        code: usize,
        register: &mut Register,
        opcodes: &mut Opcodes,
        interrupt: &mut Interrupt,
    ) -> Box<dyn CpuStepState> {
        opcodes.get(code).next_func(0, register, interrupt)
    }

    fn name(&self) -> &'static str {
        "Accumulator"
    }
}
