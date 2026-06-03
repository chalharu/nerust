// Copyright (c) 2018 chalharu
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

macro_rules! cpu_step_state_impl {
    ($name:ident) => {
        impl super::super::CpuStepState for $name {
            fn exec(
                core: &mut super::super::Core,
                ppu: &mut super::super::Ppu,
                cartridge: &mut dyn super::CpuCartridgeBus,
                controller: &mut dyn super::super::Controller,
                apu: &mut super::super::Apu,
            ) -> super::super::CpuStepStateEnum {
                Self::exec_opcode(core, ppu, cartridge, controller, apu)
            }
        }
    };
}

macro_rules! accumulate {
    ($name:ident, $getter:expr, $setter:expr, $calc:expr) => {
        pub(crate) struct $name;

        impl super::Accumulate for $name {
            fn getter(register: &super::super::Register) -> u8 {
                $getter(register)
            }

            fn setter(register: &mut super::super::Register, value: u8) {
                $setter(register, value);
            }

            fn calculator(register: &mut super::super::Register, value: u8) -> u8 {
                $calc(register, value)
            }
        }

        cpu_step_state_impl!($name);
    };
}

macro_rules! accumulate_memory {
    ($name:ident, $calc:expr) => {
        pub(crate) struct $name;

        impl AccumulateMemory for $name {
            fn calculator(register: &mut super::super::Register, value: u8) -> u8 {
                $calc(register, value)
            }
        }

        cpu_step_state_impl!($name);
    };
}

pub(super) mod arithmetic;
pub(super) mod bit;
pub(super) mod combined;
pub(super) mod compare;
pub(super) mod condition_jump;
pub(super) mod decrement;
pub(super) mod flag_control;
pub(super) mod increment;
pub(super) mod interrupt;
pub(super) mod jump;
pub(super) mod load;
pub(super) mod nop;
pub(super) mod rmw;
pub(super) mod shift;
pub(super) mod stack;
pub(super) mod store;
pub(super) mod transfer;

use super::CpuCartridgeBus;
use super::{
    Apu, Controller, Core, CpuStatesEnum, CpuStepStateEnum, Ppu, Register, read_dummy_current,
};

fn exit_opcode(core: &mut Core) -> CpuStepStateEnum {
    CpuStepStateEnum::Exit(if core.interrupt.executing {
        CpuStatesEnum::Irq
    } else {
        CpuStatesEnum::FetchOpCode
    })
}

pub(crate) trait Accumulate {
    fn getter(register: &Register) -> u8;
    fn setter(register: &mut Register, value: u8);
    fn calculator(register: &mut Register, value: u8) -> u8;

    fn exec_opcode(
        core: &mut Core,
        ppu: &mut Ppu,
        cartridge: &mut dyn CpuCartridgeBus,
        controller: &mut dyn Controller,
        apu: &mut Apu,
    ) -> CpuStepStateEnum {
        match core.internal_stat.get_step() {
            1 => {
                // dummy read
                read_dummy_current(core, ppu, cartridge, controller, apu);
                let data = Self::getter(&core.register);
                let result = Self::calculator(&mut core.register, data);
                Self::setter(&mut core.register, result);
            }
            _ => {
                return exit_opcode(core);
            }
        }
        CpuStepStateEnum::Continue
    }
}

pub(crate) trait AccumulateMemory {
    fn calculator(register: &mut Register, value: u8) -> u8;

    fn exec_opcode(
        core: &mut Core,
        ppu: &mut Ppu,
        cartridge: &mut dyn CpuCartridgeBus,
        controller: &mut dyn Controller,
        apu: &mut Apu,
    ) -> CpuStepStateEnum {
        match core.internal_stat.get_step() {
            1 => {
                core.internal_stat.set_data(core.memory.read(
                    core.internal_stat.get_address(),
                    ppu,
                    cartridge,
                    controller,
                    apu,
                    &mut core.interrupt,
                ));
            }
            2 => {
                let data = core.internal_stat.get_data();
                let result = (Self::calculator)(&mut core.register, data);
                core.internal_stat.set_data(result);
                // dummy write
                core.memory.write(
                    core.internal_stat.get_address(),
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
                    core.internal_stat.get_address(),
                    core.internal_stat.get_data(),
                    ppu,
                    cartridge,
                    controller,
                    apu,
                    &mut core.interrupt,
                );
            }
            _ => {
                return exit_opcode(core);
            }
        }
        CpuStepStateEnum::Continue
    }
}

pub(crate) struct Opcodes([CpuStatesEnum; 256]);

impl Opcodes {
    pub(crate) fn new() -> Self {
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

    pub(crate) fn get(&self, code: usize) -> CpuStatesEnum {
        self.0[code]
    }
}

impl Default for Opcodes {
    fn default() -> Self {
        Self::new()
    }
}
