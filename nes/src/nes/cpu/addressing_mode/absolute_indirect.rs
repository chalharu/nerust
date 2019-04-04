// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use super::*;

pub(crate) struct AbsoluteIndirect {
    ind_address: usize,
    address_low: u8,
}

impl AbsoluteIndirect {
    pub fn new() -> Self {
        Self {
            ind_address: 0,
            address_low: 0,
        }
    }
}

impl CpuStepState for AbsoluteIndirect {
    fn exec(
        &mut self,
        core: &mut Core,
        ppu: &mut Ppu,
        cartridge: &mut Cartridge,
        controller: &mut Controller,
        apu: &mut Apu,
    ) -> CpuStepStateEnum {
        match core.register.get_opstep() {
            1 => {
                self.address_low = core.memory.read_next(
                    &mut core.register,
                    ppu,
                    cartridge,
                    controller,
                    apu,
                    &mut core.interrupt,
                );
            }
            2 => {
                let address_high = usize::from(core.memory.read_next(
                    &mut core.register,
                    ppu,
                    cartridge,
                    controller,
                    apu,
                    &mut core.interrupt,
                ));
                self.ind_address = (address_high << 8) | usize::from(self.address_low);
            }
            3 => {
                self.address_low = core.memory.read(
                    self.ind_address,
                    ppu,
                    cartridge,
                    controller,
                    apu,
                    &mut core.interrupt,
                );
            }
            4 => {
                let address_high = usize::from(core.memory.read(
                    (self.ind_address.wrapping_add(1) & 0xFF) | (self.ind_address & 0xFF00),
                    ppu,
                    cartridge,
                    controller,
                    apu,
                    &mut core.interrupt,
                ));
                core.register
                    .set_opaddr((address_high << 8) | usize::from(self.address_low));
            }
            _ => {
                return exit_addressing_mode(core);
            }
        }
        CpuStepStateEnum::Continue
    }
}
