// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

macro_rules! cpu_step_state_impl {
    ($name:ident) => {
        impl CpuStepState for $name {
            fn exec(
                &mut self,
                core: &mut Core,
                ppu: &mut Ppu,
                cartridge: &mut Cartridge,
                controller: &mut Controller,
                apu: &mut Apu,
            ) -> CpuStepStateEnum {
                self.exec_opcode(core, ppu, cartridge, controller, apu)
            }
        }
    };
}

macro_rules! accumulate {
    ($name:ident, $getter:expr, $setter:expr, $calc:expr) => {
        pub(crate) struct $name;

        impl $name {
            pub fn new() -> Self {
                Self
            }
        }

        impl Accumulate for $name {
            fn getter(register: &Register) -> u8 {
                $getter(register)
            }

            fn setter(register: &mut Register, value: u8) {
                $setter(register, value);
            }

            fn calculator(register: &mut Register, value: u8) -> u8 {
                $calc(register, value)
            }
        }

        cpu_step_state_impl!($name);
    };
}

macro_rules! accumulate_memory {
    ($name:ident, $calc:expr) => {
        pub(crate) struct $name {
            data: u8,
        }

        impl $name {
            pub fn new() -> Self {
                Self { data: 0 }
            }
        }

        impl AccumulateMemory for $name {
            fn load(&self) -> u8 {
                self.data
            }

            fn store(&mut self, value: u8) {
                self.data = value;
            }

            fn calculator(register: &mut Register, value: u8) -> u8 {
                $calc(register, value)
            }
        }

        cpu_step_state_impl!($name);
    };
}

mod arithmetic;
mod bit;
mod combined;
mod compare;
mod condition_jump;
mod decrement;
mod flag_control;
mod increment;
pub mod interrupt;
mod jump;
mod load;
mod nop;
mod rmw;
mod shift;
mod stack;
mod store;
mod transfer;

pub(crate) use self::arithmetic::*;
pub(crate) use self::bit::*;
pub(crate) use self::combined::*;
pub(crate) use self::compare::*;
pub(crate) use self::condition_jump::*;
pub(crate) use self::decrement::*;
pub(crate) use self::flag_control::*;
pub(crate) use self::increment::*;
pub(crate) use self::interrupt::*;
pub(crate) use self::jump::*;
pub(crate) use self::load::*;
pub(crate) use self::nop::*;
pub(crate) use self::rmw::*;
pub(crate) use self::shift::*;
pub(crate) use self::stack::*;
pub(crate) use self::store::*;
pub(crate) use self::transfer::*;
use super::*;

pub(crate) trait Accumulate {
    fn getter(register: &Register) -> u8;
    fn setter(register: &mut Register, value: u8);
    fn calculator(register: &mut Register, value: u8) -> u8;

    fn exec_opcode(
        &mut self,
        core: &mut Core,
        ppu: &mut Ppu,
        cartridge: &mut Cartridge,
        controller: &mut Controller,
        apu: &mut Apu,
    ) -> CpuStepStateEnum {
        match core.register.get_opstep() {
            1 => {
                // dummy read
                read_dummy_current(core, ppu, cartridge, controller, apu);
                let data = Self::getter(&core.register);
                let result = Self::calculator(&mut core.register, data);
                Self::setter(&mut core.register, result);
            }
            _ => {
                return CpuStepStateEnum::Exit;
            }
        }
        CpuStepStateEnum::Continue
    }
}

pub(crate) trait AccumulateMemory {
    fn load(&self) -> u8;
    fn store(&mut self, value: u8);
    fn calculator(register: &mut Register, value: u8) -> u8;

    fn exec_opcode(
        &mut self,
        core: &mut Core,
        ppu: &mut Ppu,
        cartridge: &mut Cartridge,
        controller: &mut Controller,
        apu: &mut Apu,
    ) -> CpuStepStateEnum {
        match core.register.get_opstep() {
            1 => {
                self.store(core.memory.read(
                    core.register.get_opaddr(),
                    ppu,
                    cartridge,
                    controller,
                    apu,
                    &mut core.interrupt,
                ));
            }
            2 => {
                let data = self.load();
                let result = (Self::calculator)(&mut core.register, data);
                self.store(result);
                // dummy write
                core.memory.write(
                    core.register.get_opaddr(),
                    data,
                    ppu,
                    cartridge,
                    controller,
                    apu,
                    &mut core.interrupt,
                );
            }
            3 => {
                core.memory.write(
                    core.register.get_opaddr(),
                    self.load(),
                    ppu,
                    cartridge,
                    controller,
                    apu,
                    &mut core.interrupt,
                );
            }
            _ => {
                return CpuStepStateEnum::Exit;
            }
        }
        CpuStepStateEnum::Continue
    }
}

pub(crate) struct Opcodes([CpuStatesEnum; 256]);

impl Opcodes {
    pub fn new() -> Self {
        Opcodes([
            // 0x00
            CpuStatesEnum::Brk,
            CpuStatesEnum::Ora,
            CpuStatesEnum::Kil,
            CpuStatesEnum::Slo,
            CpuStatesEnum::Nop,
            CpuStatesEnum::Ora,
            CpuStatesEnum::AslMem,
            CpuStatesEnum::Slo,
            // 0x08
            CpuStatesEnum::Php,
            CpuStatesEnum::Ora,
            CpuStatesEnum::AslAcc,
            CpuStatesEnum::Anc,
            CpuStatesEnum::Nop,
            CpuStatesEnum::Ora,
            CpuStatesEnum::AslMem,
            CpuStatesEnum::Slo,
            // 0x10
            CpuStatesEnum::Bpl,
            CpuStatesEnum::Ora,
            CpuStatesEnum::Kil,
            CpuStatesEnum::Slo,
            CpuStatesEnum::Nop,
            CpuStatesEnum::Ora,
            CpuStatesEnum::AslMem,
            CpuStatesEnum::Slo,
            // 0x18
            CpuStatesEnum::Clc,
            CpuStatesEnum::Ora,
            CpuStatesEnum::Nop,
            CpuStatesEnum::Slo,
            CpuStatesEnum::Nop,
            CpuStatesEnum::Ora,
            CpuStatesEnum::AslMem,
            CpuStatesEnum::Slo,
            // 0x20
            CpuStatesEnum::Jsr,
            CpuStatesEnum::And,
            CpuStatesEnum::Kil,
            CpuStatesEnum::Rla,
            CpuStatesEnum::Bit,
            CpuStatesEnum::And,
            CpuStatesEnum::RolMem,
            CpuStatesEnum::Rla,
            // 0x28
            CpuStatesEnum::Plp,
            CpuStatesEnum::And,
            CpuStatesEnum::RolAcc,
            CpuStatesEnum::Anc,
            CpuStatesEnum::Bit,
            CpuStatesEnum::And,
            CpuStatesEnum::RolMem,
            CpuStatesEnum::Rla,
            // 0x30
            CpuStatesEnum::Bmi,
            CpuStatesEnum::And,
            CpuStatesEnum::Kil,
            CpuStatesEnum::Rla,
            CpuStatesEnum::Nop,
            CpuStatesEnum::And,
            CpuStatesEnum::RolMem,
            CpuStatesEnum::Rla,
            // 0x38
            CpuStatesEnum::Sec,
            CpuStatesEnum::And,
            CpuStatesEnum::Nop,
            CpuStatesEnum::Rla,
            CpuStatesEnum::Nop,
            CpuStatesEnum::And,
            CpuStatesEnum::RolMem,
            CpuStatesEnum::Rla,
            // 0x40
            CpuStatesEnum::Rti,
            CpuStatesEnum::Eor,
            CpuStatesEnum::Kil,
            CpuStatesEnum::Sre,
            CpuStatesEnum::Nop,
            CpuStatesEnum::Eor,
            CpuStatesEnum::LsrMem,
            CpuStatesEnum::Sre,
            // 0x48
            CpuStatesEnum::Pha,
            CpuStatesEnum::Eor,
            CpuStatesEnum::LsrAcc,
            CpuStatesEnum::Alr,
            CpuStatesEnum::Jmp,
            CpuStatesEnum::Eor,
            CpuStatesEnum::LsrMem,
            CpuStatesEnum::Sre,
            // 0x50
            CpuStatesEnum::Bvc,
            CpuStatesEnum::Eor,
            CpuStatesEnum::Kil,
            CpuStatesEnum::Sre,
            CpuStatesEnum::Nop,
            CpuStatesEnum::Eor,
            CpuStatesEnum::LsrMem,
            CpuStatesEnum::Sre,
            // 0x58
            CpuStatesEnum::Cli,
            CpuStatesEnum::Eor,
            CpuStatesEnum::Nop,
            CpuStatesEnum::Sre,
            CpuStatesEnum::Nop,
            CpuStatesEnum::Eor,
            CpuStatesEnum::LsrMem,
            CpuStatesEnum::Sre,
            // 0x60
            CpuStatesEnum::Rts,
            CpuStatesEnum::Adc,
            CpuStatesEnum::Kil,
            CpuStatesEnum::Rra,
            CpuStatesEnum::Nop,
            CpuStatesEnum::Adc,
            CpuStatesEnum::RorMem,
            CpuStatesEnum::Rra,
            // 0x68
            CpuStatesEnum::Pla,
            CpuStatesEnum::Adc,
            CpuStatesEnum::RorAcc,
            CpuStatesEnum::Arr,
            CpuStatesEnum::Jmp,
            CpuStatesEnum::Adc,
            CpuStatesEnum::RorMem,
            CpuStatesEnum::Rra,
            // 0x70
            CpuStatesEnum::Bvs,
            CpuStatesEnum::Adc,
            CpuStatesEnum::Kil,
            CpuStatesEnum::Rra,
            CpuStatesEnum::Nop,
            CpuStatesEnum::Adc,
            CpuStatesEnum::RorMem,
            CpuStatesEnum::Rra,
            // 0x78
            CpuStatesEnum::Sei,
            CpuStatesEnum::Adc,
            CpuStatesEnum::Nop,
            CpuStatesEnum::Rra,
            CpuStatesEnum::Nop,
            CpuStatesEnum::Adc,
            CpuStatesEnum::RorMem,
            CpuStatesEnum::Rra,
            // 0x80
            CpuStatesEnum::Nop,
            CpuStatesEnum::Sta,
            CpuStatesEnum::Nop,
            CpuStatesEnum::Sax,
            CpuStatesEnum::Sty,
            CpuStatesEnum::Sta,
            CpuStatesEnum::Stx,
            CpuStatesEnum::Sax,
            // 0x88
            CpuStatesEnum::Dey,
            CpuStatesEnum::Nop,
            CpuStatesEnum::Txa,
            CpuStatesEnum::Xaa,
            CpuStatesEnum::Sty,
            CpuStatesEnum::Sta,
            CpuStatesEnum::Stx,
            CpuStatesEnum::Sax,
            // 0x90
            CpuStatesEnum::Bcc,
            CpuStatesEnum::Sta,
            CpuStatesEnum::Kil,
            CpuStatesEnum::Ahx,
            CpuStatesEnum::Sty,
            CpuStatesEnum::Sta,
            CpuStatesEnum::Stx,
            CpuStatesEnum::Sax,
            // 0x98
            CpuStatesEnum::Tya,
            CpuStatesEnum::Sta,
            CpuStatesEnum::Txs,
            CpuStatesEnum::Tas,
            CpuStatesEnum::Shy,
            CpuStatesEnum::Sta,
            CpuStatesEnum::Shx,
            CpuStatesEnum::Ahx,
            // 0xA0
            CpuStatesEnum::Ldy,
            CpuStatesEnum::Lda,
            CpuStatesEnum::Ldx,
            CpuStatesEnum::Lax,
            CpuStatesEnum::Ldy,
            CpuStatesEnum::Lda,
            CpuStatesEnum::Ldx,
            CpuStatesEnum::Lax,
            // 0xA8
            CpuStatesEnum::Tay,
            CpuStatesEnum::Lda,
            CpuStatesEnum::Tax,
            CpuStatesEnum::Lax,
            CpuStatesEnum::Ldy,
            CpuStatesEnum::Lda,
            CpuStatesEnum::Ldx,
            CpuStatesEnum::Lax,
            // 0xB0
            CpuStatesEnum::Bcs,
            CpuStatesEnum::Lda,
            CpuStatesEnum::Kil,
            CpuStatesEnum::Lax,
            CpuStatesEnum::Ldy,
            CpuStatesEnum::Lda,
            CpuStatesEnum::Ldx,
            CpuStatesEnum::Lax,
            // 0xB8
            CpuStatesEnum::Clv,
            CpuStatesEnum::Lda,
            CpuStatesEnum::Tsx,
            CpuStatesEnum::Las,
            CpuStatesEnum::Ldy,
            CpuStatesEnum::Lda,
            CpuStatesEnum::Ldx,
            CpuStatesEnum::Lax,
            // 0xC0
            CpuStatesEnum::Cpy,
            CpuStatesEnum::Cmp,
            CpuStatesEnum::Nop,
            CpuStatesEnum::Dcp,
            CpuStatesEnum::Cpy,
            CpuStatesEnum::Cmp,
            CpuStatesEnum::Dec,
            CpuStatesEnum::Dcp,
            // 0xC8
            CpuStatesEnum::Iny,
            CpuStatesEnum::Cmp,
            CpuStatesEnum::Dex,
            CpuStatesEnum::Axs,
            CpuStatesEnum::Cpy,
            CpuStatesEnum::Cmp,
            CpuStatesEnum::Dec,
            CpuStatesEnum::Dcp,
            // 0xD0
            CpuStatesEnum::Bne,
            CpuStatesEnum::Cmp,
            CpuStatesEnum::Kil,
            CpuStatesEnum::Dcp,
            CpuStatesEnum::Nop,
            CpuStatesEnum::Cmp,
            CpuStatesEnum::Dec,
            CpuStatesEnum::Dcp,
            // 0xD8
            CpuStatesEnum::Cld,
            CpuStatesEnum::Cmp,
            CpuStatesEnum::Nop,
            CpuStatesEnum::Dcp,
            CpuStatesEnum::Nop,
            CpuStatesEnum::Cmp,
            CpuStatesEnum::Dec,
            CpuStatesEnum::Dcp,
            // 0xE0
            CpuStatesEnum::Cpx,
            CpuStatesEnum::Sbc,
            CpuStatesEnum::Nop,
            CpuStatesEnum::Isc,
            CpuStatesEnum::Cpx,
            CpuStatesEnum::Sbc,
            CpuStatesEnum::Inc,
            CpuStatesEnum::Isc,
            // 0xE8
            CpuStatesEnum::Inx,
            CpuStatesEnum::Sbc,
            CpuStatesEnum::Nop,
            CpuStatesEnum::Sbc,
            CpuStatesEnum::Cpx,
            CpuStatesEnum::Sbc,
            CpuStatesEnum::Inc,
            CpuStatesEnum::Isc,
            // 0xF0
            CpuStatesEnum::Beq,
            CpuStatesEnum::Sbc,
            CpuStatesEnum::Kil,
            CpuStatesEnum::Isc,
            CpuStatesEnum::Nop,
            CpuStatesEnum::Sbc,
            CpuStatesEnum::Inc,
            CpuStatesEnum::Isc,
            // 0xF8
            CpuStatesEnum::Sed,
            CpuStatesEnum::Sbc,
            CpuStatesEnum::Nop,
            CpuStatesEnum::Isc,
            CpuStatesEnum::Nop,
            CpuStatesEnum::Sbc,
            CpuStatesEnum::Inc,
            CpuStatesEnum::Isc,
        ])
    }

    pub fn get(&self, code: usize) -> CpuStatesEnum {
        self.0[code]
    }
}
