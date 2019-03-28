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
                self.interrupt.executing = self.interrupt.detected;
                let mut cpu_states = ::std::mem::replace(&mut self.cpu_states, None);
                cpu_states
                    .as_mut()
                    .unwrap()
                    .next(self, ppu, cartridge, controller, apu);
                self.cpu_states = cpu_states;
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
    Exit,
}

pub(crate) trait CpuStepState {
    fn entry(
        &mut self,
        core: &mut Core,
        ppu: &mut Ppu,
        cartridge: &mut Cartridge,
        controller: &mut Controller,
        apu: &mut Apu,
    );

    fn exec(
        &mut self,
        core: &mut Core,
        ppu: &mut Ppu,
        cartridge: &mut Cartridge,
        controller: &mut Controller,
        apu: &mut Apu,
    ) -> CpuStepStateEnum;

    fn exit(
        &mut self,
        core: &mut Core,
        _ppu: &mut Ppu,
        _cartridge: &mut Cartridge,
        _controller: &mut Controller,
        _apu: &mut Apu,
    ) -> CpuStatesEnum {
        if core.interrupt.executing {
            CpuStatesEnum::Irq
        } else {
            CpuStatesEnum::FetchOpCode
        }
    }
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
        map.insert(CpuStatesEnum::Reset, Box::new(Reset::new()));
        map.insert(CpuStatesEnum::FetchOpCode, Box::new(FetchOpCode::new()));
        map.insert(CpuStatesEnum::Irq, Box::new(Irq::new()));
        map.insert(CpuStatesEnum::AbsoluteIndirect, Box::new(AbsoluteIndirect::new()));
        map.insert(CpuStatesEnum::AbsoluteXRMW, Box::new(AbsoluteXRMW::new()));
        map.insert(CpuStatesEnum::AbsoluteX, Box::new(AbsoluteX::new()));
        map.insert(CpuStatesEnum::AbsoluteYRMW, Box::new(AbsoluteYRMW::new()));
        map.insert(CpuStatesEnum::AbsoluteY, Box::new(AbsoluteY::new()));
        map.insert(CpuStatesEnum::Absolute, Box::new(Absolute::new()));
        map.insert(CpuStatesEnum::Accumulator, Box::new(Accumulator::new()));
        map.insert(CpuStatesEnum::Immediate, Box::new(Immediate::new()));
        map.insert(CpuStatesEnum::Implied, Box::new(Implied::new()));
        map.insert(CpuStatesEnum::IndexedIndirect, Box::new(IndexedIndirect::new()));
        map.insert(CpuStatesEnum::IndirectIndexedRMW, Box::new(IndirectIndexedRMW::new()));
        map.insert(CpuStatesEnum::IndirectIndexed, Box::new(IndirectIndexed::new()));
        map.insert(CpuStatesEnum::Relative, Box::new(Relative::new()));
        map.insert(CpuStatesEnum::ZeroPageX, Box::new(ZeroPageX::new()));
        map.insert(CpuStatesEnum::ZeroPageY, Box::new(ZeroPageY::new()));
        map.insert(CpuStatesEnum::ZeroPage, Box::new(ZeroPage::new()));
        map.insert(CpuStatesEnum::And, Box::new(And::new()));
        map.insert(CpuStatesEnum::Eor, Box::new(Eor::new()));
        map.insert(CpuStatesEnum::Ora, Box::new(Ora::new()));
        map.insert(CpuStatesEnum::Adc, Box::new(Adc::new()));
        map.insert(CpuStatesEnum::Sbc, Box::new(Sbc::new()));
        map.insert(CpuStatesEnum::Bit, Box::new(Bit::new()));
        map.insert(CpuStatesEnum::Lax, Box::new(Lax::new()));
        map.insert(CpuStatesEnum::Anc, Box::new(Anc::new()));
        map.insert(CpuStatesEnum::Alr, Box::new(Alr::new()));
        map.insert(CpuStatesEnum::Arr, Box::new(Arr::new()));
        map.insert(CpuStatesEnum::Xaa, Box::new(Xaa::new()));
        map.insert(CpuStatesEnum::Las, Box::new(Las::new()));
        map.insert(CpuStatesEnum::Axs, Box::new(Axs::new()));
        map.insert(CpuStatesEnum::Sax, Box::new(Sax::new()));
        map.insert(CpuStatesEnum::Tas, Box::new(Tas::new()));
        map.insert(CpuStatesEnum::Ahx, Box::new(Ahx::new()));
        map.insert(CpuStatesEnum::Shx, Box::new(Shx::new()));
        map.insert(CpuStatesEnum::Shy, Box::new(Shy::new()));
        map.insert(CpuStatesEnum::Cmp, Box::new(Cmp::new()));
        map.insert(CpuStatesEnum::Cpx, Box::new(Cpx::new()));
        map.insert(CpuStatesEnum::Cpy, Box::new(Cpy::new()));
        map.insert(CpuStatesEnum::Bcc, Box::new(Bcc::new()));
        map.insert(CpuStatesEnum::Bcs, Box::new(Bcs::new()));
        map.insert(CpuStatesEnum::Beq, Box::new(Beq::new()));
        map.insert(CpuStatesEnum::Bmi, Box::new(Bmi::new()));
        map.insert(CpuStatesEnum::Bne, Box::new(Bne::new()));
        map.insert(CpuStatesEnum::Bpl, Box::new(Bpl::new()));
        map.insert(CpuStatesEnum::Bvc, Box::new(Bvc::new()));
        map.insert(CpuStatesEnum::Bvs, Box::new(Bvs::new()));
        map.insert(CpuStatesEnum::Dex, Box::new(Dex::new()));
        map.insert(CpuStatesEnum::Dey, Box::new(Dey::new()));
        map.insert(CpuStatesEnum::Dec, Box::new(Dec::new()));
        map.insert(CpuStatesEnum::Clc, Box::new(Clc::new()));
        map.insert(CpuStatesEnum::Cld, Box::new(Cld::new()));
        map.insert(CpuStatesEnum::Cli, Box::new(Cli::new()));
        map.insert(CpuStatesEnum::Clv, Box::new(Clv::new()));
        map.insert(CpuStatesEnum::Sec, Box::new(Sec::new()));
        map.insert(CpuStatesEnum::Sed, Box::new(Sed::new()));
        map.insert(CpuStatesEnum::Sei, Box::new(Sei::new()));
        map.insert(CpuStatesEnum::Inx, Box::new(Inx::new()));
        map.insert(CpuStatesEnum::Iny, Box::new(Iny::new()));
        map.insert(CpuStatesEnum::Inc, Box::new(Inc::new()));
        map.insert(CpuStatesEnum::Brk, Box::new(Brk::new()));
        map.insert(CpuStatesEnum::Rti, Box::new(Rti::new()));
        map.insert(CpuStatesEnum::Rts, Box::new(Rts::new()));
        map.insert(CpuStatesEnum::Jmp, Box::new(Jmp::new()));
        map.insert(CpuStatesEnum::Jsr, Box::new(Jsr::new()));
        map.insert(CpuStatesEnum::Lda, Box::new(Lda::new()));
        map.insert(CpuStatesEnum::Ldx, Box::new(Ldx::new()));
        map.insert(CpuStatesEnum::Ldy, Box::new(Ldy::new()));
        map.insert(CpuStatesEnum::Nop, Box::new(Nop::new()));
        map.insert(CpuStatesEnum::Kil, Box::new(Kil::new()));
        map.insert(CpuStatesEnum::Isc, Box::new(Isc::new()));
        map.insert(CpuStatesEnum::Dcp, Box::new(Dcp::new()));
        map.insert(CpuStatesEnum::Slo, Box::new(Slo::new()));
        map.insert(CpuStatesEnum::Rla, Box::new(Rla::new()));
        map.insert(CpuStatesEnum::Sre, Box::new(Sre::new()));
        map.insert(CpuStatesEnum::Rra, Box::new(Rra::new()));
        map.insert(CpuStatesEnum::AslAcc, Box::new(AslAcc::new()));
        map.insert(CpuStatesEnum::AslMem, Box::new(AslMem::new()));
        map.insert(CpuStatesEnum::LsrAcc, Box::new(LsrAcc::new()));
        map.insert(CpuStatesEnum::LsrMem, Box::new(LsrMem::new()));
        map.insert(CpuStatesEnum::RolAcc, Box::new(RolAcc::new()));
        map.insert(CpuStatesEnum::RolMem, Box::new(RolMem::new()));
        map.insert(CpuStatesEnum::RorAcc, Box::new(RorAcc::new()));
        map.insert(CpuStatesEnum::RorMem, Box::new(RorMem::new()));
        map.insert(CpuStatesEnum::Pla, Box::new(Pla::new()));
        map.insert(CpuStatesEnum::Plp, Box::new(Plp::new()));
        map.insert(CpuStatesEnum::Pha, Box::new(Pha::new()));
        map.insert(CpuStatesEnum::Php, Box::new(Php::new()));
        map.insert(CpuStatesEnum::Sta, Box::new(Sta::new()));
        map.insert(CpuStatesEnum::Stx, Box::new(Stx::new()));
        map.insert(CpuStatesEnum::Sty, Box::new(Sty::new()));
        map.insert(CpuStatesEnum::Tax, Box::new(Tax::new()));
        map.insert(CpuStatesEnum::Tay, Box::new(Tay::new()));
        map.insert(CpuStatesEnum::Tsx, Box::new(Tsx::new()));
        map.insert(CpuStatesEnum::Txa, Box::new(Txa::new()));
        map.insert(CpuStatesEnum::Tya, Box::new(Tya::new()));
        map.insert(CpuStatesEnum::Txs, Box::new(Txs::new()));
        let map = CpuStatesEnum::iter().map(|x| map.remove(&x).unwrap()).collect();
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
        while let CpuStepStateEnum::Exit = machine.exec(core, ppu, cartridge, controller, apu) {
            self.state = machine.exit(core, ppu, cartridge, controller, apu);
            machine = &mut self.map[self.state as usize];
            machine.entry(core, ppu, cartridge, controller, apu);
        }
    }
}

struct FetchOpCode {
    step: usize,
}

impl FetchOpCode {
    pub fn new() -> Self {
        Self { step: 0 }
    }
}

impl CpuStepState for FetchOpCode {
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
        if self.step == 0 {
            let code = usize::from(core.memory.read_next(
                &mut core.register,
                ppu,
                cartridge,
                controller,
                apu,
                &mut core.interrupt,
            ));
            core.register.set_opcode(code);
            self.step += 1;
            CpuStepStateEnum::Continue
        } else {
            CpuStepStateEnum::Exit
        }
    }

    fn exit(
        &mut self,
        core: &mut Core,
        _ppu: &mut Ppu,
        _cartridge: &mut Cartridge,
        _controller: &mut Controller,
        _apu: &mut Apu,
    ) -> CpuStatesEnum {
        core.addressing_tables.get(core.register.get_opcode())
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
