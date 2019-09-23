// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

mod addressing_mode;
mod internal_stat;
pub(crate) mod interrupt;
mod memory;
mod oamdma;
mod opcodes;
mod register;

use self::addressing_mode::*;
use self::internal_stat::{CpuStatesEnum, InternalStat};
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

#[derive(serde_derive::Serialize, serde_derive::Deserialize)]
pub(crate) struct Core {
    #[serde(skip)]
    opcode_tables: Opcodes,
    #[serde(skip)]
    addressing_tables: AddressingModeLut,
    memory: Memory,
    register: Register,
    internal_stat: InternalStat,
    interrupt: Interrupt,
    cycles: u64,
    oam_dma: Option<OamDmaState>,
    #[serde(skip, default = "make_cpu_stepfunc")]
    cpu_stepfunc: Vec<CpuStepStateFunc>,
}

// pub(crate) struct State {
//     memory: Memory,
//     register: Register,
//     internal_stat: InternalStat,
//     interrupt: Interrupt,
//     cycles: u64,
//     oam_dma: OamDmaState,
//     cpu_states: CpuStates,
// }

fn make_cpu_stepfunc() -> Vec<CpuStepStateFunc> {
    let funcs: [(CpuStatesEnum, CpuStepStateFunc); 98] = [
        (CpuStatesEnum::Reset, Reset::exec),
        (CpuStatesEnum::FetchOpCode, FetchOpCode::exec),
        (CpuStatesEnum::Irq, Irq::exec),
        (CpuStatesEnum::AbsoluteIndirect, AbsoluteIndirect::exec),
        (CpuStatesEnum::AbsoluteXRMW, AbsoluteXRMW::exec),
        (CpuStatesEnum::AbsoluteX, AbsoluteX::exec),
        (CpuStatesEnum::AbsoluteYRMW, AbsoluteYRMW::exec),
        (CpuStatesEnum::AbsoluteY, AbsoluteY::exec),
        (CpuStatesEnum::Absolute, Absolute::exec),
        (CpuStatesEnum::Accumulator, Accumulator::exec),
        (CpuStatesEnum::Immediate, Immediate::exec),
        (CpuStatesEnum::Implied, Implied::exec),
        (CpuStatesEnum::IndexedIndirect, IndexedIndirect::exec),
        (CpuStatesEnum::IndirectIndexedRMW, IndirectIndexedRMW::exec),
        (CpuStatesEnum::IndirectIndexed, IndirectIndexed::exec),
        (CpuStatesEnum::Relative, Relative::exec),
        (CpuStatesEnum::ZeroPageX, ZeroPageX::exec),
        (CpuStatesEnum::ZeroPageY, ZeroPageY::exec),
        (CpuStatesEnum::ZeroPage, ZeroPage::exec),
        (CpuStatesEnum::And, And::exec),
        (CpuStatesEnum::Eor, Eor::exec),
        (CpuStatesEnum::Ora, Ora::exec),
        (CpuStatesEnum::Adc, Adc::exec),
        (CpuStatesEnum::Sbc, Sbc::exec),
        (CpuStatesEnum::Bit, Bit::exec),
        (CpuStatesEnum::Lax, Lax::exec),
        (CpuStatesEnum::Anc, Anc::exec),
        (CpuStatesEnum::Alr, Alr::exec),
        (CpuStatesEnum::Arr, Arr::exec),
        (CpuStatesEnum::Xaa, Xaa::exec),
        (CpuStatesEnum::Las, Las::exec),
        (CpuStatesEnum::Axs, Axs::exec),
        (CpuStatesEnum::Sax, Sax::exec),
        (CpuStatesEnum::Tas, Tas::exec),
        (CpuStatesEnum::Ahx, Ahx::exec),
        (CpuStatesEnum::Shx, Shx::exec),
        (CpuStatesEnum::Shy, Shy::exec),
        (CpuStatesEnum::Cmp, Cmp::exec),
        (CpuStatesEnum::Cpx, Cpx::exec),
        (CpuStatesEnum::Cpy, Cpy::exec),
        (CpuStatesEnum::Bcc, Bcc::exec),
        (CpuStatesEnum::Bcs, Bcs::exec),
        (CpuStatesEnum::Beq, Beq::exec),
        (CpuStatesEnum::Bmi, Bmi::exec),
        (CpuStatesEnum::Bne, Bne::exec),
        (CpuStatesEnum::Bpl, Bpl::exec),
        (CpuStatesEnum::Bvc, Bvc::exec),
        (CpuStatesEnum::Bvs, Bvs::exec),
        (CpuStatesEnum::Dex, Dex::exec),
        (CpuStatesEnum::Dey, Dey::exec),
        (CpuStatesEnum::Dec, Dec::exec),
        (CpuStatesEnum::Clc, Clc::exec),
        (CpuStatesEnum::Cld, Cld::exec),
        (CpuStatesEnum::Cli, Cli::exec),
        (CpuStatesEnum::Clv, Clv::exec),
        (CpuStatesEnum::Sec, Sec::exec),
        (CpuStatesEnum::Sed, Sed::exec),
        (CpuStatesEnum::Sei, Sei::exec),
        (CpuStatesEnum::Inx, Inx::exec),
        (CpuStatesEnum::Iny, Iny::exec),
        (CpuStatesEnum::Inc, Inc::exec),
        (CpuStatesEnum::Brk, Brk::exec),
        (CpuStatesEnum::Rti, Rti::exec),
        (CpuStatesEnum::Rts, Rts::exec),
        (CpuStatesEnum::Jmp, Jmp::exec),
        (CpuStatesEnum::Jsr, Jsr::exec),
        (CpuStatesEnum::Lda, Lda::exec),
        (CpuStatesEnum::Ldx, Ldx::exec),
        (CpuStatesEnum::Ldy, Ldy::exec),
        (CpuStatesEnum::Nop, Nop::exec),
        (CpuStatesEnum::Kil, Kil::exec),
        (CpuStatesEnum::Isc, Isc::exec),
        (CpuStatesEnum::Dcp, Dcp::exec),
        (CpuStatesEnum::Slo, Slo::exec),
        (CpuStatesEnum::Rla, Rla::exec),
        (CpuStatesEnum::Sre, Sre::exec),
        (CpuStatesEnum::Rra, Rra::exec),
        (CpuStatesEnum::AslAcc, AslAcc::exec),
        (CpuStatesEnum::AslMem, AslMem::exec),
        (CpuStatesEnum::LsrAcc, LsrAcc::exec),
        (CpuStatesEnum::LsrMem, LsrMem::exec),
        (CpuStatesEnum::RolAcc, RolAcc::exec),
        (CpuStatesEnum::RolMem, RolMem::exec),
        (CpuStatesEnum::RorAcc, RorAcc::exec),
        (CpuStatesEnum::RorMem, RorMem::exec),
        (CpuStatesEnum::Pla, Pla::exec),
        (CpuStatesEnum::Plp, Plp::exec),
        (CpuStatesEnum::Pha, Pha::exec),
        (CpuStatesEnum::Php, Php::exec),
        (CpuStatesEnum::Sta, Sta::exec),
        (CpuStatesEnum::Stx, Stx::exec),
        (CpuStatesEnum::Sty, Sty::exec),
        (CpuStatesEnum::Tax, Tax::exec),
        (CpuStatesEnum::Tay, Tay::exec),
        (CpuStatesEnum::Tsx, Tsx::exec),
        (CpuStatesEnum::Txa, Txa::exec),
        (CpuStatesEnum::Tya, Tya::exec),
        (CpuStatesEnum::Txs, Txs::exec),
    ];
    let mut map: HashMap<CpuStatesEnum, CpuStepStateFunc> = funcs
        .iter()
        .cloned()
        .collect::<HashMap<CpuStatesEnum, CpuStepStateFunc>>();
    CpuStatesEnum::iter()
        .map(|x| map.remove(&x).unwrap())
        .collect()
}

impl Core {
    pub(crate) fn new() -> Self {
        Self {
            opcode_tables: Opcodes::new(),
            addressing_tables: AddressingModeLut::new(),
            register: Register::new(),
            internal_stat: InternalStat::new(),
            interrupt: Interrupt::new(),
            memory: Memory::new(),
            cycles: 0,
            oam_dma: Some(OamDmaState::new()),
            cpu_stepfunc: make_cpu_stepfunc(),
        }
    }

    pub(crate) fn reset(&mut self) {
        self.interrupt.reset();
        self.oam_dma.as_mut().unwrap().reset();
        self.internal_stat.reset();
        self.cycles = 0;
    }

    pub(crate) fn step(
        &mut self,
        ppu: &mut Ppu,
        cartridge: &mut dyn Cartridge,
        controller: &mut dyn Controller,
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
                let mut machine = &mut self.cpu_stepfunc[self.internal_stat.get_state() as usize];
                let step = self.internal_stat.get_step() + 1;
                self.internal_stat.set_step(step);
                while let CpuStepStateEnum::Exit(s) = machine(self, ppu, cartridge, controller, apu)
                {
                    self.internal_stat.set_state(s);
                    self.internal_stat.set_step(1);
                    machine = &mut self.cpu_stepfunc[self.internal_stat.get_state() as usize];
                }
                self.interrupt.executing = self.interrupt.detected;
                self.interrupt.detected = self.interrupt.nmi
                    || (!((self.interrupt.irq_flag & self.interrupt.irq_mask).is_empty())
                        && !self.register.get_i());
            }
        }
    }

    pub(crate) fn interrupt_mut(&mut self) -> &mut Interrupt {
        &mut self.interrupt
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
        cartridge: &mut dyn Cartridge,
        controller: &mut dyn Controller,
        apu: &mut Apu,
    ) -> CpuStepStateEnum;
}

type CpuStepStateFunc =
    fn(&mut Core, &mut Ppu, &mut dyn Cartridge, &mut dyn Controller, &mut Apu) -> CpuStepStateEnum;

struct FetchOpCode;

impl CpuStepState for FetchOpCode {
    fn exec(
        core: &mut Core,
        ppu: &mut Ppu,
        cartridge: &mut dyn Cartridge,
        controller: &mut dyn Controller,
        apu: &mut Apu,
    ) -> CpuStepStateEnum {
        if core.internal_stat.get_step() == 1 {
            let code = usize::from(core.memory.read_next(
                &mut core.register,
                ppu,
                cartridge,
                controller,
                apu,
                &mut core.interrupt,
            ));
            core.internal_stat.set_opcode(code);
            CpuStepStateEnum::Continue
        } else {
            CpuStepStateEnum::Exit(core.addressing_tables.get(core.internal_stat.get_opcode()))
        }
    }
}

fn push(
    core: &mut Core,
    ppu: &mut Ppu,
    cartridge: &mut dyn Cartridge,
    controller: &mut dyn Controller,
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
    cartridge: &mut dyn Cartridge,
    controller: &mut dyn Controller,
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
    cartridge: &mut dyn Cartridge,
    controller: &mut dyn Controller,
    apu: &mut Apu,
) {
    let pc = usize::from(core.register.get_pc());
    let _ = core
        .memory
        .read(pc, ppu, cartridge, controller, apu, &mut core.interrupt);
}
