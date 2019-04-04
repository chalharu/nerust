// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

mod addressing_mode;
pub mod interrupt;
mod memory;
mod oamdma;
mod opcodes;
mod register;

use self::addressing_mode::*;
use self::interrupt::{Interrupt, IrqSource};
use self::memory::Memory;
use self::oamdma::OamDmaState;
use self::opcodes::{
    interrupt::{Irq, Reset},
    *,
};
use self::register::{Register, RegisterP};
use super::*;
use std::collections::HashMap;
use std::ops::Shr;
use strum::IntoEnumIterator;

fn page_crossed<T: Shr<usize>>(a: T, b: T) -> bool
where
    T::Output: PartialEq,
{
    a >> 8 != b >> 8
}

const NMI_VECTOR: usize = 0xFFFA;
const RESET_VECTOR: usize = 0xFFFC;
const IRQ_VECTOR: usize = 0xFFFE;

// #[derive(Serialize, Deserialize, Debug)]
pub(crate) struct Core {
    opcode_tables: Opcodes,
    addressing_tables: AddressingModeLut,
    memory: Memory,
    register: Register,
    pub(crate) interrupt: Interrupt,
    cycles: u64,
    oam_dma: Option<OamDmaState>,
    cpu_states: Option<CpuStates>,
}

impl Core {
    pub fn new() -> Self {
        Self {
            opcode_tables: Opcodes::new(),
            addressing_tables: AddressingModeLut::new(),
            register: Register::new(),
            interrupt: Interrupt::new(),
            memory: Memory::new(),
            cycles: 0,
            oam_dma: Some(OamDmaState::new()),
            cpu_states: Some(CpuStates::new()),
        }
    }

    pub fn reset(&mut self) {
        self.interrupt.reset();
        self.oam_dma.as_mut().unwrap().reset();
        self.cpu_states.as_mut().unwrap().reset();
        self.cycles = 0;
        self.register.set_opstep(1);
    }

    pub fn step(
        &mut self,
        ppu: &mut Ppu,
        cartridge: &mut Cartridge,
        controller: &mut Controller,
        apu: &mut Apu,
    ) {
        self.cycles = self.cycles.wrapping_add(1);

        if self.interrupt.dmc_start {
            self.interrupt.dmc_start = false;
            self.interrupt.dmc_count = match self.oam_dma.as_ref().unwrap().count() {
                None => 4,
                Some(0) => 3,
                Some(1) => 1,
                _ => 2,
            };
        }

        if self.interrupt.dmc_count > 0 && (self.cycles & 1 == 0) {
            self.interrupt.dmc_count -= 1;
            if self.interrupt.dmc_count == 0 {
                if let Some(addr) = apu.dmc_fill_address() {
                    let value = self.memory.read(
                        addr,
                        ppu,
                        cartridge,
                        controller,
                        apu,
                        &mut self.interrupt,
                    );
                    apu.dmc_fill(value, &mut self.interrupt);
                }
            }
        } else {
            if let Some(offset) = ::std::mem::replace(&mut self.interrupt.oam_dma, None) {
                self.oam_dma.as_mut().unwrap().start_transaction(offset);
            }

            if self.oam_dma.as_ref().unwrap().has_transaction() {
                let mut oam_dma = ::std::mem::replace(&mut self.oam_dma, None);
                oam_dma
                    .as_mut()
                    .unwrap()
                    .next(self, ppu, cartridge, controller, apu);
                self.oam_dma = oam_dma;
            } else {
                let mut cpu_states = ::std::mem::replace(&mut self.cpu_states, None);
                cpu_states
                    .as_mut()
                    .unwrap()
                    .next(self, ppu, cartridge, controller, apu);
                self.cpu_states = cpu_states;
                self.interrupt.executing = self.interrupt.detected;
                self.interrupt.detected = self.interrupt.nmi
                    || (!((self.interrupt.irq_flag & self.interrupt.irq_mask).is_empty())
                        && !self.register.get_i());
            }
        }
    }
}

#[derive(PartialEq, Eq, Clone, Copy, Hash)]
pub(crate) enum CpuStepStateEnum {
    Continue,
    Exit(CpuStatesEnum),
}

pub(crate) trait CpuStepState {
    fn exec(
        &mut self,
        core: &mut Core,
        ppu: &mut Ppu,
        cartridge: &mut Cartridge,
        controller: &mut Controller,
        apu: &mut Apu,
    ) -> CpuStepStateEnum;
}

#[derive(PartialEq, Eq, Clone, Copy, Hash, Debug, EnumIter)]
pub(crate) enum CpuStatesEnum {
    FetchOpCode,
    Reset,
    Irq,
    AbsoluteIndirect,
    AbsoluteXRMW,
    AbsoluteX,
    AbsoluteYRMW,
    AbsoluteY,
    Absolute,
    Accumulator,
    Immediate,
    Implied,
    IndexedIndirect,
    IndirectIndexedRMW,
    IndirectIndexed,
    Relative,
    ZeroPageX,
    ZeroPageY,
    ZeroPage,
    And,
    Eor,
    Ora,
    Adc,
    Sbc,
    Bit,
    Lax,
    Anc,
    Alr,
    Arr,
    Xaa,
    Las,
    Axs,
    Sax,
    Tas,
    Ahx,
    Shx,
    Shy,
    Cmp,
    Cpx,
    Cpy,
    Bcc,
    Bcs,
    Beq,
    Bmi,
    Bne,
    Bpl,
    Bvc,
    Bvs,
    Dex,
    Dey,
    Dec,
    Clc,
    Cld,
    Cli,
    Clv,
    Sec,
    Sed,
    Sei,
    Inx,
    Iny,
    Inc,
    Brk,
    Rti,
    Rts,
    Jmp,
    Jsr,
    Lda,
    Ldx,
    Ldy,
    Nop,
    Kil,
    Isc,
    Dcp,
    Slo,
    Rla,
    Sre,
    Rra,
    AslAcc,
    AslMem,
    LsrAcc,
    LsrMem,
    RolAcc,
    RolMem,
    RorAcc,
    RorMem,
    Pla,
    Plp,
    Pha,
    Php,
    Sta,
    Stx,
    Sty,
    Tax,
    Tay,
    Tsx,
    Txa,
    Tya,
    Txs,
}

pub(crate) struct CpuStates {
    state: CpuStatesEnum,
    map: Vec<Box<CpuStepState>>,
}

impl CpuStates {
    pub fn new() -> Self {
        let mut map: HashMap<CpuStatesEnum, Box<CpuStepState>> = HashMap::new();
        map.insert(CpuStatesEnum::Reset, Box::new(Reset));
        map.insert(CpuStatesEnum::FetchOpCode, Box::new(FetchOpCode));
        map.insert(CpuStatesEnum::Irq, Box::new(Irq));
        map.insert(CpuStatesEnum::AbsoluteIndirect, Box::new(AbsoluteIndirect));
        map.insert(CpuStatesEnum::AbsoluteXRMW, Box::new(AbsoluteXRMW));
        map.insert(CpuStatesEnum::AbsoluteX, Box::new(AbsoluteX));
        map.insert(CpuStatesEnum::AbsoluteYRMW, Box::new(AbsoluteYRMW));
        map.insert(CpuStatesEnum::AbsoluteY, Box::new(AbsoluteY));
        map.insert(CpuStatesEnum::Absolute, Box::new(Absolute));
        map.insert(CpuStatesEnum::Accumulator, Box::new(Accumulator));
        map.insert(CpuStatesEnum::Immediate, Box::new(Immediate));
        map.insert(CpuStatesEnum::Implied, Box::new(Implied));
        map.insert(CpuStatesEnum::IndexedIndirect, Box::new(IndexedIndirect));
        map.insert(
            CpuStatesEnum::IndirectIndexedRMW,
            Box::new(IndirectIndexedRMW),
        );
        map.insert(CpuStatesEnum::IndirectIndexed, Box::new(IndirectIndexed));
        map.insert(CpuStatesEnum::Relative, Box::new(Relative));
        map.insert(CpuStatesEnum::ZeroPageX, Box::new(ZeroPageX));
        map.insert(CpuStatesEnum::ZeroPageY, Box::new(ZeroPageY));
        map.insert(CpuStatesEnum::ZeroPage, Box::new(ZeroPage));
        map.insert(CpuStatesEnum::And, Box::new(And));
        map.insert(CpuStatesEnum::Eor, Box::new(Eor));
        map.insert(CpuStatesEnum::Ora, Box::new(Ora));
        map.insert(CpuStatesEnum::Adc, Box::new(Adc));
        map.insert(CpuStatesEnum::Sbc, Box::new(Sbc));
        map.insert(CpuStatesEnum::Bit, Box::new(Bit));
        map.insert(CpuStatesEnum::Lax, Box::new(Lax));
        map.insert(CpuStatesEnum::Anc, Box::new(Anc));
        map.insert(CpuStatesEnum::Alr, Box::new(Alr));
        map.insert(CpuStatesEnum::Arr, Box::new(Arr));
        map.insert(CpuStatesEnum::Xaa, Box::new(Xaa));
        map.insert(CpuStatesEnum::Las, Box::new(Las));
        map.insert(CpuStatesEnum::Axs, Box::new(Axs));
        map.insert(CpuStatesEnum::Sax, Box::new(Sax));
        map.insert(CpuStatesEnum::Tas, Box::new(Tas));
        map.insert(CpuStatesEnum::Ahx, Box::new(Ahx));
        map.insert(CpuStatesEnum::Shx, Box::new(Shx));
        map.insert(CpuStatesEnum::Shy, Box::new(Shy));
        map.insert(CpuStatesEnum::Cmp, Box::new(Cmp));
        map.insert(CpuStatesEnum::Cpx, Box::new(Cpx));
        map.insert(CpuStatesEnum::Cpy, Box::new(Cpy));
        map.insert(CpuStatesEnum::Bcc, Box::new(Bcc));
        map.insert(CpuStatesEnum::Bcs, Box::new(Bcs));
        map.insert(CpuStatesEnum::Beq, Box::new(Beq));
        map.insert(CpuStatesEnum::Bmi, Box::new(Bmi));
        map.insert(CpuStatesEnum::Bne, Box::new(Bne));
        map.insert(CpuStatesEnum::Bpl, Box::new(Bpl));
        map.insert(CpuStatesEnum::Bvc, Box::new(Bvc));
        map.insert(CpuStatesEnum::Bvs, Box::new(Bvs));
        map.insert(CpuStatesEnum::Dex, Box::new(Dex));
        map.insert(CpuStatesEnum::Dey, Box::new(Dey));
        map.insert(CpuStatesEnum::Dec, Box::new(Dec));
        map.insert(CpuStatesEnum::Clc, Box::new(Clc));
        map.insert(CpuStatesEnum::Cld, Box::new(Cld));
        map.insert(CpuStatesEnum::Cli, Box::new(Cli));
        map.insert(CpuStatesEnum::Clv, Box::new(Clv));
        map.insert(CpuStatesEnum::Sec, Box::new(Sec));
        map.insert(CpuStatesEnum::Sed, Box::new(Sed));
        map.insert(CpuStatesEnum::Sei, Box::new(Sei));
        map.insert(CpuStatesEnum::Inx, Box::new(Inx));
        map.insert(CpuStatesEnum::Iny, Box::new(Iny));
        map.insert(CpuStatesEnum::Inc, Box::new(Inc));
        map.insert(CpuStatesEnum::Brk, Box::new(Brk));
        map.insert(CpuStatesEnum::Rti, Box::new(Rti));
        map.insert(CpuStatesEnum::Rts, Box::new(Rts));
        map.insert(CpuStatesEnum::Jmp, Box::new(Jmp));
        map.insert(CpuStatesEnum::Jsr, Box::new(Jsr));
        map.insert(CpuStatesEnum::Lda, Box::new(Lda));
        map.insert(CpuStatesEnum::Ldx, Box::new(Ldx));
        map.insert(CpuStatesEnum::Ldy, Box::new(Ldy));
        map.insert(CpuStatesEnum::Nop, Box::new(Nop));
        map.insert(CpuStatesEnum::Kil, Box::new(Kil));
        map.insert(CpuStatesEnum::Isc, Box::new(Isc));
        map.insert(CpuStatesEnum::Dcp, Box::new(Dcp));
        map.insert(CpuStatesEnum::Slo, Box::new(Slo));
        map.insert(CpuStatesEnum::Rla, Box::new(Rla));
        map.insert(CpuStatesEnum::Sre, Box::new(Sre));
        map.insert(CpuStatesEnum::Rra, Box::new(Rra));
        map.insert(CpuStatesEnum::AslAcc, Box::new(AslAcc));
        map.insert(CpuStatesEnum::AslMem, Box::new(AslMem));
        map.insert(CpuStatesEnum::LsrAcc, Box::new(LsrAcc));
        map.insert(CpuStatesEnum::LsrMem, Box::new(LsrMem));
        map.insert(CpuStatesEnum::RolAcc, Box::new(RolAcc));
        map.insert(CpuStatesEnum::RolMem, Box::new(RolMem));
        map.insert(CpuStatesEnum::RorAcc, Box::new(RorAcc));
        map.insert(CpuStatesEnum::RorMem, Box::new(RorMem));
        map.insert(CpuStatesEnum::Pla, Box::new(Pla));
        map.insert(CpuStatesEnum::Plp, Box::new(Plp));
        map.insert(CpuStatesEnum::Pha, Box::new(Pha));
        map.insert(CpuStatesEnum::Php, Box::new(Php));
        map.insert(CpuStatesEnum::Sta, Box::new(Sta));
        map.insert(CpuStatesEnum::Stx, Box::new(Stx));
        map.insert(CpuStatesEnum::Sty, Box::new(Sty));
        map.insert(CpuStatesEnum::Tax, Box::new(Tax));
        map.insert(CpuStatesEnum::Tay, Box::new(Tay));
        map.insert(CpuStatesEnum::Tsx, Box::new(Tsx));
        map.insert(CpuStatesEnum::Txa, Box::new(Txa));
        map.insert(CpuStatesEnum::Tya, Box::new(Tya));
        map.insert(CpuStatesEnum::Txs, Box::new(Txs));
        let map = CpuStatesEnum::iter()
            .map(|x| map.remove(&x).unwrap())
            .collect();
        Self {
            state: CpuStatesEnum::Reset,
            map,
        }
    }

    pub fn reset(&mut self) {
        self.state = CpuStatesEnum::Reset;
    }

    pub fn next(
        &mut self,
        core: &mut Core,
        ppu: &mut Ppu,
        cartridge: &mut Cartridge,
        controller: &mut Controller,
        apu: &mut Apu,
    ) {
        let mut machine = &mut self.map[self.state as usize];
        let step = core.register.get_opstep() + 1;
        core.register.set_opstep(step);
        while let CpuStepStateEnum::Exit(s) = machine.exec(core, ppu, cartridge, controller, apu) {
            self.state = s;
            core.register.set_opstep(1);
            machine = &mut self.map[self.state as usize];
        }
    }
}

struct FetchOpCode;

impl CpuStepState for FetchOpCode {
    fn exec(
        &mut self,
        core: &mut Core,
        ppu: &mut Ppu,
        cartridge: &mut Cartridge,
        controller: &mut Controller,
        apu: &mut Apu,
    ) -> CpuStepStateEnum {
        if core.register.get_opstep() == 1 {
            let code = usize::from(core.memory.read_next(
                &mut core.register,
                ppu,
                cartridge,
                controller,
                apu,
                &mut core.interrupt,
            ));
            core.register.set_opcode(code);
            CpuStepStateEnum::Continue
        } else {
            CpuStepStateEnum::Exit(core.addressing_tables.get(core.register.get_opcode()))
        }
    }
}

fn push(
    core: &mut Core,
    ppu: &mut Ppu,
    cartridge: &mut Cartridge,
    controller: &mut Controller,
    apu: &mut Apu,
    value: u8,
) {
    let sp = core.register.get_sp();
    core.register.set_sp(sp.wrapping_sub(1));
    core.memory.write(
        0x100 | usize::from(sp),
        value,
        ppu,
        cartridge,
        controller,
        apu,
        &mut core.interrupt,
    );
}

fn pull(
    core: &mut Core,
    ppu: &mut Ppu,
    cartridge: &mut Cartridge,
    controller: &mut Controller,
    apu: &mut Apu,
) -> u8 {
    let sp = core.register.get_sp().wrapping_add(1);
    core.register.set_sp(sp);
    core.memory.read(
        0x100 | usize::from(sp),
        ppu,
        cartridge,
        controller,
        apu,
        &mut core.interrupt,
    )
}

fn read_dummy_current(
    core: &mut Core,
    ppu: &mut Ppu,
    cartridge: &mut Cartridge,
    controller: &mut Controller,
    apu: &mut Apu,
) {
    let pc = usize::from(core.register.get_pc());
    let _ = core
        .memory
        .read(pc, ppu, cartridge, controller, apu, &mut core.interrupt);
}
