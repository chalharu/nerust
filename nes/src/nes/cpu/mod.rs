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

type CpuStepStateFunc =
    fn(&mut Core, &mut Ppu, &mut Cartridge, &mut Controller, &mut Apu) -> CpuStepStateEnum;

pub(crate) struct CpuStates {
    state: CpuStatesEnum,
    map: Vec<CpuStepStateFunc>,
}

impl CpuStates {
    pub fn new() -> Self {
        let mut map: HashMap<CpuStatesEnum, CpuStepStateFunc> = HashMap::new();
        map.insert(CpuStatesEnum::Reset, Reset::exec);
        map.insert(CpuStatesEnum::FetchOpCode, FetchOpCode::exec);
        map.insert(CpuStatesEnum::Irq, Irq::exec);
        map.insert(CpuStatesEnum::AbsoluteIndirect, AbsoluteIndirect::exec);
        map.insert(CpuStatesEnum::AbsoluteXRMW, AbsoluteXRMW::exec);
        map.insert(CpuStatesEnum::AbsoluteX, AbsoluteX::exec);
        map.insert(CpuStatesEnum::AbsoluteYRMW, AbsoluteYRMW::exec);
        map.insert(CpuStatesEnum::AbsoluteY, AbsoluteY::exec);
        map.insert(CpuStatesEnum::Absolute, Absolute::exec);
        map.insert(CpuStatesEnum::Accumulator, Accumulator::exec);
        map.insert(CpuStatesEnum::Immediate, Immediate::exec);
        map.insert(CpuStatesEnum::Implied, Implied::exec);
        map.insert(CpuStatesEnum::IndexedIndirect, IndexedIndirect::exec);
        map.insert(CpuStatesEnum::IndirectIndexedRMW, IndirectIndexedRMW::exec);
        map.insert(CpuStatesEnum::IndirectIndexed, IndirectIndexed::exec);
        map.insert(CpuStatesEnum::Relative, Relative::exec);
        map.insert(CpuStatesEnum::ZeroPageX, ZeroPageX::exec);
        map.insert(CpuStatesEnum::ZeroPageY, ZeroPageY::exec);
        map.insert(CpuStatesEnum::ZeroPage, ZeroPage::exec);
        map.insert(CpuStatesEnum::And, And::exec);
        map.insert(CpuStatesEnum::Eor, Eor::exec);
        map.insert(CpuStatesEnum::Ora, Ora::exec);
        map.insert(CpuStatesEnum::Adc, Adc::exec);
        map.insert(CpuStatesEnum::Sbc, Sbc::exec);
        map.insert(CpuStatesEnum::Bit, Bit::exec);
        map.insert(CpuStatesEnum::Lax, Lax::exec);
        map.insert(CpuStatesEnum::Anc, Anc::exec);
        map.insert(CpuStatesEnum::Alr, Alr::exec);
        map.insert(CpuStatesEnum::Arr, Arr::exec);
        map.insert(CpuStatesEnum::Xaa, Xaa::exec);
        map.insert(CpuStatesEnum::Las, Las::exec);
        map.insert(CpuStatesEnum::Axs, Axs::exec);
        map.insert(CpuStatesEnum::Sax, Sax::exec);
        map.insert(CpuStatesEnum::Tas, Tas::exec);
        map.insert(CpuStatesEnum::Ahx, Ahx::exec);
        map.insert(CpuStatesEnum::Shx, Shx::exec);
        map.insert(CpuStatesEnum::Shy, Shy::exec);
        map.insert(CpuStatesEnum::Cmp, Cmp::exec);
        map.insert(CpuStatesEnum::Cpx, Cpx::exec);
        map.insert(CpuStatesEnum::Cpy, Cpy::exec);
        map.insert(CpuStatesEnum::Bcc, Bcc::exec);
        map.insert(CpuStatesEnum::Bcs, Bcs::exec);
        map.insert(CpuStatesEnum::Beq, Beq::exec);
        map.insert(CpuStatesEnum::Bmi, Bmi::exec);
        map.insert(CpuStatesEnum::Bne, Bne::exec);
        map.insert(CpuStatesEnum::Bpl, Bpl::exec);
        map.insert(CpuStatesEnum::Bvc, Bvc::exec);
        map.insert(CpuStatesEnum::Bvs, Bvs::exec);
        map.insert(CpuStatesEnum::Dex, Dex::exec);
        map.insert(CpuStatesEnum::Dey, Dey::exec);
        map.insert(CpuStatesEnum::Dec, Dec::exec);
        map.insert(CpuStatesEnum::Clc, Clc::exec);
        map.insert(CpuStatesEnum::Cld, Cld::exec);
        map.insert(CpuStatesEnum::Cli, Cli::exec);
        map.insert(CpuStatesEnum::Clv, Clv::exec);
        map.insert(CpuStatesEnum::Sec, Sec::exec);
        map.insert(CpuStatesEnum::Sed, Sed::exec);
        map.insert(CpuStatesEnum::Sei, Sei::exec);
        map.insert(CpuStatesEnum::Inx, Inx::exec);
        map.insert(CpuStatesEnum::Iny, Iny::exec);
        map.insert(CpuStatesEnum::Inc, Inc::exec);
        map.insert(CpuStatesEnum::Brk, Brk::exec);
        map.insert(CpuStatesEnum::Rti, Rti::exec);
        map.insert(CpuStatesEnum::Rts, Rts::exec);
        map.insert(CpuStatesEnum::Jmp, Jmp::exec);
        map.insert(CpuStatesEnum::Jsr, Jsr::exec);
        map.insert(CpuStatesEnum::Lda, Lda::exec);
        map.insert(CpuStatesEnum::Ldx, Ldx::exec);
        map.insert(CpuStatesEnum::Ldy, Ldy::exec);
        map.insert(CpuStatesEnum::Nop, Nop::exec);
        map.insert(CpuStatesEnum::Kil, Kil::exec);
        map.insert(CpuStatesEnum::Isc, Isc::exec);
        map.insert(CpuStatesEnum::Dcp, Dcp::exec);
        map.insert(CpuStatesEnum::Slo, Slo::exec);
        map.insert(CpuStatesEnum::Rla, Rla::exec);
        map.insert(CpuStatesEnum::Sre, Sre::exec);
        map.insert(CpuStatesEnum::Rra, Rra::exec);
        map.insert(CpuStatesEnum::AslAcc, AslAcc::exec);
        map.insert(CpuStatesEnum::AslMem, AslMem::exec);
        map.insert(CpuStatesEnum::LsrAcc, LsrAcc::exec);
        map.insert(CpuStatesEnum::LsrMem, LsrMem::exec);
        map.insert(CpuStatesEnum::RolAcc, RolAcc::exec);
        map.insert(CpuStatesEnum::RolMem, RolMem::exec);
        map.insert(CpuStatesEnum::RorAcc, RorAcc::exec);
        map.insert(CpuStatesEnum::RorMem, RorMem::exec);
        map.insert(CpuStatesEnum::Pla, Pla::exec);
        map.insert(CpuStatesEnum::Plp, Plp::exec);
        map.insert(CpuStatesEnum::Pha, Pha::exec);
        map.insert(CpuStatesEnum::Php, Php::exec);
        map.insert(CpuStatesEnum::Sta, Sta::exec);
        map.insert(CpuStatesEnum::Stx, Stx::exec);
        map.insert(CpuStatesEnum::Sty, Sty::exec);
        map.insert(CpuStatesEnum::Tax, Tax::exec);
        map.insert(CpuStatesEnum::Tay, Tay::exec);
        map.insert(CpuStatesEnum::Tsx, Tsx::exec);
        map.insert(CpuStatesEnum::Txa, Txa::exec);
        map.insert(CpuStatesEnum::Tya, Tya::exec);
        map.insert(CpuStatesEnum::Txs, Txs::exec);
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
        while let CpuStepStateEnum::Exit(s) = machine(core, ppu, cartridge, controller, apu) {
            self.state = s;
            core.register.set_opstep(1);
            machine = &mut self.map[self.state as usize];
        }
    }
}

struct FetchOpCode;

impl CpuStepState for FetchOpCode {
    fn exec(
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
