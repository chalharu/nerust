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
use self::interrupt::{DmcDmaKind, Interrupt, IrqSource};
use self::memory::Memory;
use self::oamdma::OamDmaState;
use self::opcodes::{
    interrupt::{Irq, Reset},
    *,
};
use self::register::{Register, RegisterP};
use super::*;
use std::ops::Shr;

fn page_crossed<T: Shr<usize>>(a: T, b: T) -> bool
where
    T::Output: PartialEq,
{
    a >> 8 != b >> 8
}

const NMI_VECTOR: usize = 0xFFFA;
const RESET_VECTOR: usize = 0xFFFC;
const IRQ_VECTOR: usize = 0xFFFE;

#[derive(serde_derive::Serialize, serde_derive::Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
enum DmcDmaPhase {
    WaitForHalt,
    Dummy,
    Align,
    Read,
}

#[derive(serde_derive::Serialize, serde_derive::Deserialize, Debug, Clone, Copy)]
struct DmcDmaState {
    delay: u8,
    halt_on_get_cycle: bool,
    halted_on_get_cycle: bool,
    attempted_halt: bool,
    phase: DmcDmaPhase,
}

impl DmcDmaState {
    fn from_kind(kind: DmcDmaKind) -> Self {
        match kind {
            DmcDmaKind::Load => Self {
                delay: 2,
                halt_on_get_cycle: true,
                halted_on_get_cycle: false,
                attempted_halt: false,
                phase: DmcDmaPhase::WaitForHalt,
            },
            DmcDmaKind::Reload => Self {
                delay: 0,
                halt_on_get_cycle: false,
                halted_on_get_cycle: false,
                attempted_halt: false,
                phase: DmcDmaPhase::WaitForHalt,
            },
        }
    }
}

#[derive(serde_derive::Serialize)]
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
    dmc_dma: Option<DmcDmaState>,
    #[serde(skip)]
    cpu_stepfunc: CpuStepStateFunc,
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

macro_rules! cpu_stepfunc_entries {
    ($with_entries:ident) => {
        $with_entries! {
            (CpuStatesEnum::FetchOpCode, FetchOpCode::exec),
            (CpuStatesEnum::Reset, Reset::exec),
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
        }
    };
}

macro_rules! cpu_stepfunc_array {
    ($(($state:expr, $func:path)),+ $(,)?) => {
        [$($func),+]
    };
}

const CPU_STEPFUNCS: [CpuStepStateFunc; CpuStatesEnum::COUNT] =
    cpu_stepfunc_entries!(cpu_stepfunc_array);

fn cpu_stepfunc(state: CpuStatesEnum) -> CpuStepStateFunc {
    CPU_STEPFUNCS[state as usize]
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
            dmc_dma: None,
            cpu_stepfunc: cpu_stepfunc(CpuStatesEnum::Reset),
        }
    }

    pub(crate) fn reset(&mut self) {
        self.interrupt.reset();
        self.oam_dma.as_mut().unwrap().reset();
        self.internal_stat.reset();
        self.cpu_stepfunc = cpu_stepfunc(self.internal_stat.state);
        self.cycles = 0;
        self.dmc_dma = None;
    }

    pub(crate) fn peek_work_ram(&self, address: usize) -> Option<u8> {
        self.memory.peek_work_ram(address)
    }

    fn set_cpu_state(&mut self, state: CpuStatesEnum) {
        self.internal_stat.state = state;
        self.cpu_stepfunc = cpu_stepfunc(state);
    }

    pub(crate) fn step(
        &mut self,
        ppu: &mut Ppu,
        cartridge: &mut dyn Cartridge,
        controller: &mut dyn Controller,
        apu: &mut Apu,
    ) {
        self.cycles = self.cycles.wrapping_add(1);

        if let Some(offset) = self.interrupt.oam_dma.take() {
            self.oam_dma.as_mut().unwrap().start_transaction(offset);
        }

        if let Some(kind) = self.interrupt.dmc_dma_request.take() {
            self.dmc_dma = Some(DmcDmaState::from_kind(kind));
        }

        if self.process_dma_cycle(ppu, cartridge, controller, apu) {
            return;
        }

        let mut machine = self.cpu_stepfunc;
        self.internal_stat.step += 1;
        while let CpuStepStateEnum::Exit(s) = machine(self, ppu, cartridge, controller, apu) {
            self.set_cpu_state(s);
            self.internal_stat.step = 1;
            machine = self.cpu_stepfunc;
        }
        self.interrupt.executing = self.interrupt.detected;
        self.interrupt.detected = self.interrupt.nmi
            || (!((self.interrupt.irq_flag & self.interrupt.irq_mask).is_empty())
                && !self.register.get_i());
    }

    pub(crate) fn interrupt_mut(&mut self) -> &mut Interrupt {
        &mut self.interrupt
    }

    fn process_dma_cycle(
        &mut self,
        ppu: &mut Ppu,
        cartridge: &mut dyn Cartridge,
        controller: &mut dyn Controller,
        apu: &mut Apu,
    ) -> bool {
        let oam_active = self.oam_dma.as_ref().unwrap().has_transaction();
        let cpu_write_cycle = self.current_cpu_cycle_is_write();
        let is_get_cycle = self.cycles & 1 != 0;

        if let Some(mut dmc_dma) = self.dmc_dma {
            if dmc_dma.delay > 0 {
                dmc_dma.delay -= 1;
                self.dmc_dma = Some(dmc_dma);
                if oam_active {
                    self.advance_oam_dma(ppu, cartridge, controller, apu);
                    return true;
                }
                return false;
            }

            match dmc_dma.phase {
                DmcDmaPhase::WaitForHalt => {
                    let can_attempt_halt =
                        dmc_dma.attempted_halt || is_get_cycle == dmc_dma.halt_on_get_cycle;

                    if can_attempt_halt && (oam_active || !cpu_write_cycle) {
                        dmc_dma.halted_on_get_cycle = is_get_cycle;
                        dmc_dma.phase = DmcDmaPhase::Dummy;
                        dmc_dma.attempted_halt = false;
                        self.dmc_dma = Some(dmc_dma);
                        if oam_active {
                            self.advance_oam_dma(ppu, cartridge, controller, apu);
                        } else {
                            read_dummy_current(self, ppu, cartridge, controller, apu);
                        }
                        return true;
                    }

                    if can_attempt_halt && cpu_write_cycle {
                        dmc_dma.attempted_halt = true;
                    }

                    self.dmc_dma = Some(dmc_dma);
                    if oam_active {
                        self.advance_oam_dma(ppu, cartridge, controller, apu);
                        return true;
                    }
                    return false;
                }
                DmcDmaPhase::Dummy => {
                    dmc_dma.phase = if dmc_dma.halted_on_get_cycle {
                        DmcDmaPhase::Read
                    } else {
                        DmcDmaPhase::Align
                    };
                    self.dmc_dma = Some(dmc_dma);
                    if oam_active {
                        self.advance_oam_dma(ppu, cartridge, controller, apu);
                    } else {
                        read_dummy_current(self, ppu, cartridge, controller, apu);
                    }
                    return true;
                }
                DmcDmaPhase::Align => {
                    dmc_dma.phase = DmcDmaPhase::Read;
                    self.dmc_dma = Some(dmc_dma);
                    if oam_active {
                        self.advance_oam_dma(ppu, cartridge, controller, apu);
                    } else {
                        read_dummy_current(self, ppu, cartridge, controller, apu);
                    }
                    return true;
                }
                DmcDmaPhase::Read => {
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
                    self.dmc_dma = None;
                    return true;
                }
            }
        }

        if oam_active {
            self.advance_oam_dma(ppu, cartridge, controller, apu);
            return true;
        }

        false
    }

    fn advance_oam_dma(
        &mut self,
        ppu: &mut Ppu,
        cartridge: &mut dyn Cartridge,
        controller: &mut dyn Controller,
        apu: &mut Apu,
    ) {
        let mut oam_dma = self.oam_dma.take();
        oam_dma
            .as_mut()
            .unwrap()
            .next(self, ppu, cartridge, controller, apu);
        self.oam_dma = oam_dma;
    }

    fn current_cpu_cycle_is_write(&self) -> bool {
        let step = self.internal_stat.get_step();
        let state = self.internal_stat.get_state();

        if self.addressing_cycle_exits_into_store_write(state, step) {
            return true;
        }

        match state {
            CpuStatesEnum::Dec
            | CpuStatesEnum::Inc
            | CpuStatesEnum::Isc
            | CpuStatesEnum::Dcp
            | CpuStatesEnum::Slo
            | CpuStatesEnum::Rla
            | CpuStatesEnum::Sre
            | CpuStatesEnum::Rra
            | CpuStatesEnum::AslMem
            | CpuStatesEnum::LsrMem
            | CpuStatesEnum::RolMem
            | CpuStatesEnum::RorMem => step == 1 || step == 2,
            CpuStatesEnum::Pha | CpuStatesEnum::Php => step == 1,
            CpuStatesEnum::Jsr => step == 1 || step == 2,
            CpuStatesEnum::Brk => (1..=3).contains(&step),
            CpuStatesEnum::Irq => (2..=4).contains(&step),
            _ => false,
        }
    }

    fn addressing_cycle_exits_into_store_write(&self, state: CpuStatesEnum, step: usize) -> bool {
        let opcode_state = self.opcode_tables.get(self.internal_stat.get_opcode());
        if !matches!(
            opcode_state,
            CpuStatesEnum::Sta
                | CpuStatesEnum::Stx
                | CpuStatesEnum::Sty
                | CpuStatesEnum::Sax
                | CpuStatesEnum::Tas
                | CpuStatesEnum::Ahx
                | CpuStatesEnum::Shx
                | CpuStatesEnum::Shy
        ) {
            return false;
        }

        match state {
            CpuStatesEnum::ZeroPage => step == 1,
            CpuStatesEnum::ZeroPageX | CpuStatesEnum::ZeroPageY => step == 2,
            CpuStatesEnum::Absolute => step == 2,
            CpuStatesEnum::AbsoluteX | CpuStatesEnum::AbsoluteY => {
                step == 3
                    || (step == 2
                        && !page_crossed(
                            self.internal_stat.get_tempaddr(),
                            self.internal_stat.get_address(),
                        ))
            }
            CpuStatesEnum::IndexedIndirect => step == 4,
            CpuStatesEnum::IndirectIndexed => {
                step == 4
                    || (step == 3
                        && !page_crossed(
                            self.internal_stat.get_tempaddr(),
                            self.internal_stat.get_address(),
                        ))
            }
            _ => false,
        }
    }
}

#[derive(serde_derive::Deserialize)]
struct CoreDeserialize {
    memory: Memory,
    register: Register,
    internal_stat: InternalStat,
    interrupt: Interrupt,
    cycles: u64,
    oam_dma: Option<OamDmaState>,
    dmc_dma: Option<DmcDmaState>,
}

impl<'de> serde::Deserialize<'de> for Core {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let data = <CoreDeserialize as serde::Deserialize>::deserialize(deserializer)?;
        Ok(Self {
            opcode_tables: Opcodes::new(),
            addressing_tables: AddressingModeLut::new(),
            cpu_stepfunc: cpu_stepfunc(data.internal_stat.state),
            memory: data.memory,
            register: data.register,
            internal_stat: data.internal_stat,
            interrupt: data.interrupt,
            cycles: data.cycles,
            oam_dma: data.oam_dma,
            dmc_dma: data.dmc_dma,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use strum::IntoEnumIterator;

    macro_rules! cpu_stepfunc_pair_array {
        ($(($state:expr, $func:path)),+ $(,)?) => {
            [$(($state, $func as CpuStepStateFunc)),+]
        };
    }

    #[test]
    fn cpu_stepfunc_table_tracks_cpu_state_order() {
        assert_eq!(CpuStatesEnum::iter().count(), CpuStatesEnum::COUNT);
        assert_eq!(CPU_STEPFUNCS.len(), CpuStatesEnum::COUNT);
        let expected = cpu_stepfunc_entries!(cpu_stepfunc_pair_array);
        assert_eq!(expected.len(), CpuStatesEnum::COUNT);
        for (index, (state, expected_func)) in expected.into_iter().enumerate() {
            assert_eq!(state as usize, index);
            assert!(std::ptr::fn_addr_eq(CPU_STEPFUNCS[index], expected_func));
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
        if core.internal_stat.step == 1 {
            let code = usize::from(core.memory.read_next(
                &mut core.register,
                ppu,
                cartridge,
                controller,
                apu,
                &mut core.interrupt,
            ));
            core.internal_stat.opcode = code;
            CpuStepStateEnum::Continue
        } else {
            CpuStepStateEnum::Exit(core.addressing_tables.get(core.internal_stat.opcode))
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
