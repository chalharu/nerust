// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use super::*;

pub(crate) struct Brk {
    step: usize,
    low: u8,
}

impl Brk {
    pub fn new() -> Self {
        Self { step: 0, low: 0 }
    }
}

impl CpuStepState for Brk {
    fn entry(
        &mut self,
        _core: &mut Core,
        _ppu: &mut Ppu,
        _cartridge: &mut Cartridge,
        _controller: &mut Controller,
        _apu: &mut Apu,
    ) {
        self.step = 0;
    }

    fn exec(
        &mut self,
        core: &mut Core,
        ppu: &mut Ppu,
        cartridge: &mut Cartridge,
        controller: &mut Controller,
        apu: &mut Apu,
    ) -> CpuStepStateEnum {
        self.step += 1;
        match self.step {
            1 => {
                // dummy read
                core.memory.read_next(
                    &mut core.register,
                    ppu,
                    cartridge,
                    controller,
                    apu,
                    &mut core.interrupt,
                );
            }
            2 => {
                let pc = core.register.get_pc();
                let hi = (pc >> 8) as u8;
                self.low = (pc & 0xFF) as u8;

                push(core, ppu, cartridge, controller, apu, hi);
            }
            3 => {
                push(core, ppu, cartridge, controller, apu, self.low);

                core.register.set_opaddr(if core.interrupt.nmi {
                    // core.interrupt.nmi = false;
                    NMI_VECTOR
                } else {
                    IRQ_VECTOR
                });
            }
            4 => {
                let p = core.register.get_p() | (RegisterP::BREAK | RegisterP::RESERVED).bits();
                push(core, ppu, cartridge, controller, apu, p);
            }
            5 => {
                core.register.set_i(true);
                self.low = core.memory.read(
                    core.register.get_opaddr(),
                    ppu,
                    cartridge,
                    controller,
                    apu,
                    &mut core.interrupt,
                );
            }
            6 => {
                let hi = u16::from(core.memory.read(
                    core.register.get_opaddr() + 1,
                    ppu,
                    cartridge,
                    controller,
                    apu,
                    &mut core.interrupt,
                ));
                core.register.set_pc((hi << 8) | u16::from(self.low));
            }
            _ => {
                core.interrupt.executing = false;
                return CpuStepStateEnum::Exit;
            }
        }
        CpuStepStateEnum::Continue
    }
}

pub(crate) struct Rti {
    step: usize,
    low: u8,
}

impl Rti {
    pub fn new() -> Self {
        Self { step: 0, low: 0 }
    }
}

impl CpuStepState for Rti {
    fn entry(
        &mut self,
        _core: &mut Core,
        _ppu: &mut Ppu,
        _cartridge: &mut Cartridge,
        _controller: &mut Controller,
        _apu: &mut Apu,
    ) {
        self.step = 0;
    }

    fn exec(
        &mut self,
        core: &mut Core,
        ppu: &mut Ppu,
        cartridge: &mut Cartridge,
        controller: &mut Controller,
        apu: &mut Apu,
    ) -> CpuStepStateEnum {
        self.step += 1;
        match self.step {
            1 => {
                // dummy read
                read_dummy_current(core, ppu, cartridge, controller, apu);
            }
            2 => {
                // dummy read
                let sp = usize::from(core.register.get_sp());
                let _ = core.memory.read(
                    sp | 0x100,
                    ppu,
                    cartridge,
                    controller,
                    apu,
                    &mut core.interrupt,
                );
            }
            3 => {
                let p = pull(core, ppu, cartridge, controller, apu);
                core.register
                    .set_p((p & !(RegisterP::BREAK.bits())) | RegisterP::RESERVED.bits());
            }
            4 => {
                self.low = pull(core, ppu, cartridge, controller, apu);
            }
            5 => {
                let high = pull(core, ppu, cartridge, controller, apu);

                core.register
                    .set_pc(u16::from(self.low) | (u16::from(high) << 8));
            }
            _ => {
                return CpuStepStateEnum::Exit;
            }
        }
        CpuStepStateEnum::Continue
    }
}

pub(crate) struct Irq {
    step: usize,
    low: u8,
    nmi: bool,
}

impl Irq {
    pub fn new() -> Self {
        Self {
            step: 0,
            low: 0,
            nmi: false,
        }
    }
}

impl CpuStepState for Irq {
    fn entry(
        &mut self,
        _core: &mut Core,
        _ppu: &mut Ppu,
        _cartridge: &mut Cartridge,
        _controller: &mut Controller,
        _apu: &mut Apu,
    ) {
        self.step = 0;
    }

    fn exec(
        &mut self,
        core: &mut Core,
        ppu: &mut Ppu,
        cartridge: &mut Cartridge,
        controller: &mut Controller,
        apu: &mut Apu,
    ) -> CpuStepStateEnum {
        self.step += 1;
        match self.step {
            1 => {
                // dummy read
                read_dummy_current(core, ppu, cartridge, controller, apu);
            }
            2 => {
                // dummy read
                read_dummy_current(core, ppu, cartridge, controller, apu);
            }
            3 => {
                let pc = core.register.get_pc();
                let hi = (pc >> 8) as u8;
                self.low = (pc & 0xFF) as u8;
                push(core, ppu, cartridge, controller, apu, hi);
            }
            4 => {
                push(core, ppu, cartridge, controller, apu, self.low);
                self.nmi = core.interrupt.nmi;

                core.register.set_opaddr(if core.interrupt.nmi {
                    NMI_VECTOR
                } else {
                    IRQ_VECTOR
                });
            }
            5 => {
                let p =
                    (core.register.get_p() & !RegisterP::BREAK.bits()) | RegisterP::RESERVED.bits();
                push(core, ppu, cartridge, controller, apu, p);
            }
            6 => {
                core.register.set_i(true);
                self.low = core.memory.read(
                    core.register.get_opaddr(),
                    ppu,
                    cartridge,
                    controller,
                    apu,
                    &mut core.interrupt,
                );
                if self.nmi {
                    core.interrupt.nmi = false;
                }
            }
            7 => {
                let hi = u16::from(core.memory.read(
                    core.register.get_opaddr() + 1,
                    ppu,
                    cartridge,
                    controller,
                    apu,
                    &mut core.interrupt,
                ));
                core.register.set_pc((hi << 8) | u16::from(self.low));
            }
            _ => {
                core.interrupt.executing = false;
                return CpuStepStateEnum::Exit;
            }
        }
        CpuStepStateEnum::Continue
    }
}

pub(crate) struct Reset {
    step: usize,
    low: u8,
}

impl Reset {
    pub fn new() -> Self {
        Self { step: 0, low: 0 }
    }
}

impl CpuStepState for Reset {
    fn entry(
        &mut self,
        _core: &mut Core,
        _ppu: &mut Ppu,
        _cartridge: &mut Cartridge,
        _controller: &mut Controller,
        _apu: &mut Apu,
    ) {
    }

    fn exec(
        &mut self,
        core: &mut Core,
        ppu: &mut Ppu,
        cartridge: &mut Cartridge,
        controller: &mut Controller,
        apu: &mut Apu,
    ) -> CpuStepStateEnum {
        self.step += 1;
        match self.step {
            1 => {
                // dummy read
                read_dummy_current(core, ppu, cartridge, controller, apu);

                core.interrupt.irq_flag = IrqSource::empty();
                core.interrupt.irq_mask = IrqSource::ALL;
                core.interrupt.nmi = false;
            }
            2 => {
                // dummy read
                read_dummy_current(core, ppu, cartridge, controller, apu);
            }
            3 => {
                let sp = usize::from(core.register.get_sp());
                core.register.set_sp((sp.wrapping_sub(1) & 0xFF) as u8);
                core.memory.read(
                    0x100 | sp,
                    ppu,
                    cartridge,
                    controller,
                    apu,
                    &mut core.interrupt,
                );
            }
            4 => {
                let sp = usize::from(core.register.get_sp());
                core.register.set_sp((sp.wrapping_sub(1) & 0xFF) as u8);
                core.memory.read(
                    0x100 | sp,
                    ppu,
                    cartridge,
                    controller,
                    apu,
                    &mut core.interrupt,
                );
            }
            5 => {
                let sp = usize::from(core.register.get_sp());
                core.register.set_sp((sp.wrapping_sub(1) & 0xFF) as u8);
                core.memory.read(
                    0x100 | sp,
                    ppu,
                    cartridge,
                    controller,
                    apu,
                    &mut core.interrupt,
                );
            }
            6 => {
                core.register.set_i(true);
                self.low = core.memory.read(
                    RESET_VECTOR,
                    ppu,
                    cartridge,
                    controller,
                    apu,
                    &mut core.interrupt,
                );
            }
            7 => {
                let hi = u16::from(core.memory.read(
                    RESET_VECTOR + 1,
                    ppu,
                    cartridge,
                    controller,
                    apu,
                    &mut core.interrupt,
                ));
                core.register.set_pc((hi << 8) | u16::from(self.low));
                core.interrupt.executing = false;
            }
            _ => {
                self.step = 0;
                return CpuStepStateEnum::Exit;
            }
        }
        CpuStepStateEnum::Continue
    }
}
