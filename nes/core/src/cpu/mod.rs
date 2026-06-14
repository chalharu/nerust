mod addressing_mode;
mod internal_stat;
mod memory;
mod oamdma;
mod opcodes;
mod register;

use self::addressing_mode::{
    AddressingModeLut, absolute::Absolute, absolute_indirect::AbsoluteIndirect,
    absolute_x::AbsoluteX, absolute_x_rmw::AbsoluteXRMW, absolute_y::AbsoluteY,
    absolute_y_rmw::AbsoluteYRMW, accumulator::Accumulator, immediate::Immediate, implied::Implied,
    indexed_indirect::IndexedIndirect, indirect_indexed::IndirectIndexed,
    indirect_indexed_rmw::IndirectIndexedRMW, relative::Relative, zero_page::ZeroPage,
    zero_page_x::ZeroPageX, zero_page_y::ZeroPageY,
};
use self::internal_stat::{CpuStatesEnum, InternalStat};
use self::memory::Memory;
use self::oamdma::OamDmaState;
use self::opcodes::{
    Opcodes,
    arithmetic::{Adc, And, Eor, Ora, Sbc},
    bit::Bit,
    combined::{Ahx, Alr, Anc, Arr, Axs, Las, Lax, Sax, Shx, Shy, Tas, Xaa},
    compare::{Cmp, Cpx, Cpy},
    condition_jump::{Bcc, Bcs, Beq, Bmi, Bne, Bpl, Bvc, Bvs},
    decrement::{Dec, Dex, Dey},
    flag_control::{Clc, Cld, Cli, Clv, Sec, Sed, Sei},
    increment::{Inc, Inx, Iny},
    interrupt::{Brk, Irq, Reset, Rti},
    jump::{Jmp, Jsr, Rts},
    load::{Lda, Ldx, Ldy},
    nop::{Kil, Nop},
    rmw::{Dcp, Isc, Rla, Rra, Slo, Sre},
    shift::{AslAcc, AslMem, LsrAcc, LsrMem, RolAcc, RolMem, RorAcc, RorMem},
    stack::{Pha, Php, Pla, Plp},
    store::{Sta, Stx, Sty},
    transfer::{Tax, Tay, Tsx, Txa, Txs, Tya},
};
use self::register::{Register, RegisterP};
use crate::cart_device::Cartridge as MapperCartridge;
use crate::cartridge_bus::{CpuCartridgeBus, mapper_cartridge_bus};
use crate::interrupt::{DmcDmaKind, Interrupt, IrqSource};
use crate::persistence_error::PersistenceError;
use crate::{Apu, Controller, Ppu};
use std::ops::Shr;

use crate::cartridge_bus::CpuCartridgeBus as Cartridge;

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

#[derive(Clone, Copy)]
struct FastPathPlan {
    cycles: u64,
    kind: FastPathPlanKind,
}

#[derive(Clone, Copy)]
enum FastPathPlanKind {
    ImpliedOrAccumulator,
    Immediate,
    ReadMemory(FastPathMemoryAccess),
    StoreMemory(FastPathMemoryAccess),
    Branch(FastPathBranch),
}

#[derive(Clone, Copy)]
struct FastPathMemoryAccess {
    final_address: usize,
    next_opcode_pc: u16,
    operand_bytes: u8,
    dummy_read_address: Option<usize>,
    temp_address: Option<usize>,
    cycles: u64,
}

#[derive(Clone, Copy)]
struct FastPathBranch {
    taken: bool,
    target: u16,
    fallthrough: u16,
    crossed: bool,
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
            (crate::cpu::CpuStatesEnum::FetchOpCode, <crate::cpu::FetchOpCode as crate::cpu::CpuStepState>::exec),
            (crate::cpu::CpuStatesEnum::Reset, <crate::cpu::Reset as crate::cpu::CpuStepState>::exec),
            (crate::cpu::CpuStatesEnum::Irq, <crate::cpu::Irq as crate::cpu::CpuStepState>::exec),
            (crate::cpu::CpuStatesEnum::AbsoluteIndirect, <crate::cpu::AbsoluteIndirect as crate::cpu::CpuStepState>::exec),
            (crate::cpu::CpuStatesEnum::AbsoluteXRMW, <crate::cpu::AbsoluteXRMW as crate::cpu::CpuStepState>::exec),
            (crate::cpu::CpuStatesEnum::AbsoluteX, <crate::cpu::AbsoluteX as crate::cpu::CpuStepState>::exec),
            (crate::cpu::CpuStatesEnum::AbsoluteYRMW, <crate::cpu::AbsoluteYRMW as crate::cpu::CpuStepState>::exec),
            (crate::cpu::CpuStatesEnum::AbsoluteY, <crate::cpu::AbsoluteY as crate::cpu::CpuStepState>::exec),
            (crate::cpu::CpuStatesEnum::Absolute, <crate::cpu::Absolute as crate::cpu::CpuStepState>::exec),
            (crate::cpu::CpuStatesEnum::Accumulator, <crate::cpu::Accumulator as crate::cpu::CpuStepState>::exec),
            (crate::cpu::CpuStatesEnum::Immediate, <crate::cpu::Immediate as crate::cpu::CpuStepState>::exec),
            (crate::cpu::CpuStatesEnum::Implied, <crate::cpu::Implied as crate::cpu::CpuStepState>::exec),
            (crate::cpu::CpuStatesEnum::IndexedIndirect, <crate::cpu::IndexedIndirect as crate::cpu::CpuStepState>::exec),
            (crate::cpu::CpuStatesEnum::IndirectIndexedRMW, <crate::cpu::IndirectIndexedRMW as crate::cpu::CpuStepState>::exec),
            (crate::cpu::CpuStatesEnum::IndirectIndexed, <crate::cpu::IndirectIndexed as crate::cpu::CpuStepState>::exec),
            (crate::cpu::CpuStatesEnum::Relative, <crate::cpu::Relative as crate::cpu::CpuStepState>::exec),
            (crate::cpu::CpuStatesEnum::ZeroPageX, <crate::cpu::ZeroPageX as crate::cpu::CpuStepState>::exec),
            (crate::cpu::CpuStatesEnum::ZeroPageY, <crate::cpu::ZeroPageY as crate::cpu::CpuStepState>::exec),
            (crate::cpu::CpuStatesEnum::ZeroPage, <crate::cpu::ZeroPage as crate::cpu::CpuStepState>::exec),
            (crate::cpu::CpuStatesEnum::And, <crate::cpu::And as crate::cpu::CpuStepState>::exec),
            (crate::cpu::CpuStatesEnum::Eor, <crate::cpu::Eor as crate::cpu::CpuStepState>::exec),
            (crate::cpu::CpuStatesEnum::Ora, <crate::cpu::Ora as crate::cpu::CpuStepState>::exec),
            (crate::cpu::CpuStatesEnum::Adc, <crate::cpu::Adc as crate::cpu::CpuStepState>::exec),
            (crate::cpu::CpuStatesEnum::Sbc, <crate::cpu::Sbc as crate::cpu::CpuStepState>::exec),
            (crate::cpu::CpuStatesEnum::Bit, <crate::cpu::Bit as crate::cpu::CpuStepState>::exec),
            (crate::cpu::CpuStatesEnum::Lax, <crate::cpu::Lax as crate::cpu::CpuStepState>::exec),
            (crate::cpu::CpuStatesEnum::Anc, <crate::cpu::Anc as crate::cpu::CpuStepState>::exec),
            (crate::cpu::CpuStatesEnum::Alr, <crate::cpu::Alr as crate::cpu::CpuStepState>::exec),
            (crate::cpu::CpuStatesEnum::Arr, <crate::cpu::Arr as crate::cpu::CpuStepState>::exec),
            (crate::cpu::CpuStatesEnum::Xaa, <crate::cpu::Xaa as crate::cpu::CpuStepState>::exec),
            (crate::cpu::CpuStatesEnum::Las, <crate::cpu::Las as crate::cpu::CpuStepState>::exec),
            (crate::cpu::CpuStatesEnum::Axs, <crate::cpu::Axs as crate::cpu::CpuStepState>::exec),
            (crate::cpu::CpuStatesEnum::Sax, <crate::cpu::Sax as crate::cpu::CpuStepState>::exec),
            (crate::cpu::CpuStatesEnum::Tas, <crate::cpu::Tas as crate::cpu::CpuStepState>::exec),
            (crate::cpu::CpuStatesEnum::Ahx, <crate::cpu::Ahx as crate::cpu::CpuStepState>::exec),
            (crate::cpu::CpuStatesEnum::Shx, <crate::cpu::Shx as crate::cpu::CpuStepState>::exec),
            (crate::cpu::CpuStatesEnum::Shy, <crate::cpu::Shy as crate::cpu::CpuStepState>::exec),
            (crate::cpu::CpuStatesEnum::Cmp, <crate::cpu::Cmp as crate::cpu::CpuStepState>::exec),
            (crate::cpu::CpuStatesEnum::Cpx, <crate::cpu::Cpx as crate::cpu::CpuStepState>::exec),
            (crate::cpu::CpuStatesEnum::Cpy, <crate::cpu::Cpy as crate::cpu::CpuStepState>::exec),
            (crate::cpu::CpuStatesEnum::Bcc, <crate::cpu::Bcc as crate::cpu::CpuStepState>::exec),
            (crate::cpu::CpuStatesEnum::Bcs, <crate::cpu::Bcs as crate::cpu::CpuStepState>::exec),
            (crate::cpu::CpuStatesEnum::Beq, <crate::cpu::Beq as crate::cpu::CpuStepState>::exec),
            (crate::cpu::CpuStatesEnum::Bmi, <crate::cpu::Bmi as crate::cpu::CpuStepState>::exec),
            (crate::cpu::CpuStatesEnum::Bne, <crate::cpu::Bne as crate::cpu::CpuStepState>::exec),
            (crate::cpu::CpuStatesEnum::Bpl, <crate::cpu::Bpl as crate::cpu::CpuStepState>::exec),
            (crate::cpu::CpuStatesEnum::Bvc, <crate::cpu::Bvc as crate::cpu::CpuStepState>::exec),
            (crate::cpu::CpuStatesEnum::Bvs, <crate::cpu::Bvs as crate::cpu::CpuStepState>::exec),
            (crate::cpu::CpuStatesEnum::Dex, <crate::cpu::Dex as crate::cpu::CpuStepState>::exec),
            (crate::cpu::CpuStatesEnum::Dey, <crate::cpu::Dey as crate::cpu::CpuStepState>::exec),
            (crate::cpu::CpuStatesEnum::Dec, <crate::cpu::Dec as crate::cpu::CpuStepState>::exec),
            (crate::cpu::CpuStatesEnum::Clc, <crate::cpu::Clc as crate::cpu::CpuStepState>::exec),
            (crate::cpu::CpuStatesEnum::Cld, <crate::cpu::Cld as crate::cpu::CpuStepState>::exec),
            (crate::cpu::CpuStatesEnum::Cli, <crate::cpu::Cli as crate::cpu::CpuStepState>::exec),
            (crate::cpu::CpuStatesEnum::Clv, <crate::cpu::Clv as crate::cpu::CpuStepState>::exec),
            (crate::cpu::CpuStatesEnum::Sec, <crate::cpu::Sec as crate::cpu::CpuStepState>::exec),
            (crate::cpu::CpuStatesEnum::Sed, <crate::cpu::Sed as crate::cpu::CpuStepState>::exec),
            (crate::cpu::CpuStatesEnum::Sei, <crate::cpu::Sei as crate::cpu::CpuStepState>::exec),
            (crate::cpu::CpuStatesEnum::Inx, <crate::cpu::Inx as crate::cpu::CpuStepState>::exec),
            (crate::cpu::CpuStatesEnum::Iny, <crate::cpu::Iny as crate::cpu::CpuStepState>::exec),
            (crate::cpu::CpuStatesEnum::Inc, <crate::cpu::Inc as crate::cpu::CpuStepState>::exec),
            (crate::cpu::CpuStatesEnum::Brk, <crate::cpu::Brk as crate::cpu::CpuStepState>::exec),
            (crate::cpu::CpuStatesEnum::Rti, <crate::cpu::Rti as crate::cpu::CpuStepState>::exec),
            (crate::cpu::CpuStatesEnum::Rts, <crate::cpu::Rts as crate::cpu::CpuStepState>::exec),
            (crate::cpu::CpuStatesEnum::Jmp, <crate::cpu::Jmp as crate::cpu::CpuStepState>::exec),
            (crate::cpu::CpuStatesEnum::Jsr, <crate::cpu::Jsr as crate::cpu::CpuStepState>::exec),
            (crate::cpu::CpuStatesEnum::Lda, <crate::cpu::Lda as crate::cpu::CpuStepState>::exec),
            (crate::cpu::CpuStatesEnum::Ldx, <crate::cpu::Ldx as crate::cpu::CpuStepState>::exec),
            (crate::cpu::CpuStatesEnum::Ldy, <crate::cpu::Ldy as crate::cpu::CpuStepState>::exec),
            (crate::cpu::CpuStatesEnum::Nop, <crate::cpu::Nop as crate::cpu::CpuStepState>::exec),
            (crate::cpu::CpuStatesEnum::Kil, <crate::cpu::Kil as crate::cpu::CpuStepState>::exec),
            (crate::cpu::CpuStatesEnum::Isc, <crate::cpu::Isc as crate::cpu::CpuStepState>::exec),
            (crate::cpu::CpuStatesEnum::Dcp, <crate::cpu::Dcp as crate::cpu::CpuStepState>::exec),
            (crate::cpu::CpuStatesEnum::Slo, <crate::cpu::Slo as crate::cpu::CpuStepState>::exec),
            (crate::cpu::CpuStatesEnum::Rla, <crate::cpu::Rla as crate::cpu::CpuStepState>::exec),
            (crate::cpu::CpuStatesEnum::Sre, <crate::cpu::Sre as crate::cpu::CpuStepState>::exec),
            (crate::cpu::CpuStatesEnum::Rra, <crate::cpu::Rra as crate::cpu::CpuStepState>::exec),
            (crate::cpu::CpuStatesEnum::AslAcc, <crate::cpu::AslAcc as crate::cpu::CpuStepState>::exec),
            (crate::cpu::CpuStatesEnum::AslMem, <crate::cpu::AslMem as crate::cpu::CpuStepState>::exec),
            (crate::cpu::CpuStatesEnum::LsrAcc, <crate::cpu::LsrAcc as crate::cpu::CpuStepState>::exec),
            (crate::cpu::CpuStatesEnum::LsrMem, <crate::cpu::LsrMem as crate::cpu::CpuStepState>::exec),
            (crate::cpu::CpuStatesEnum::RolAcc, <crate::cpu::RolAcc as crate::cpu::CpuStepState>::exec),
            (crate::cpu::CpuStatesEnum::RolMem, <crate::cpu::RolMem as crate::cpu::CpuStepState>::exec),
            (crate::cpu::CpuStatesEnum::RorAcc, <crate::cpu::RorAcc as crate::cpu::CpuStepState>::exec),
            (crate::cpu::CpuStatesEnum::RorMem, <crate::cpu::RorMem as crate::cpu::CpuStepState>::exec),
            (crate::cpu::CpuStatesEnum::Pla, <crate::cpu::Pla as crate::cpu::CpuStepState>::exec),
            (crate::cpu::CpuStatesEnum::Plp, <crate::cpu::Plp as crate::cpu::CpuStepState>::exec),
            (crate::cpu::CpuStatesEnum::Pha, <crate::cpu::Pha as crate::cpu::CpuStepState>::exec),
            (crate::cpu::CpuStatesEnum::Php, <crate::cpu::Php as crate::cpu::CpuStepState>::exec),
            (crate::cpu::CpuStatesEnum::Sta, <crate::cpu::Sta as crate::cpu::CpuStepState>::exec),
            (crate::cpu::CpuStatesEnum::Stx, <crate::cpu::Stx as crate::cpu::CpuStepState>::exec),
            (crate::cpu::CpuStatesEnum::Sty, <crate::cpu::Sty as crate::cpu::CpuStepState>::exec),
            (crate::cpu::CpuStatesEnum::Tax, <crate::cpu::Tax as crate::cpu::CpuStepState>::exec),
            (crate::cpu::CpuStatesEnum::Tay, <crate::cpu::Tay as crate::cpu::CpuStepState>::exec),
            (crate::cpu::CpuStatesEnum::Tsx, <crate::cpu::Tsx as crate::cpu::CpuStepState>::exec),
            (crate::cpu::CpuStatesEnum::Txa, <crate::cpu::Txa as crate::cpu::CpuStepState>::exec),
            (crate::cpu::CpuStatesEnum::Tya, <crate::cpu::Tya as crate::cpu::CpuStepState>::exec),
            (crate::cpu::CpuStatesEnum::Txs, <crate::cpu::Txs as crate::cpu::CpuStepState>::exec),
        }
    };
}

macro_rules! cpu_stepfunc_array {
    ($(($state:expr, $func:expr)),+ $(,)?) => {
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
        cartridge: &mut dyn MapperCartridge,
        controller: &mut dyn Controller,
        apu: &mut Apu,
    ) {
        let mut cartridge = mapper_cartridge_bus(cartridge);
        self.cycles = self.cycles.wrapping_add(1);

        if let Some(offset) = self.interrupt.oam_dma.take() {
            self.oam_dma.as_mut().unwrap().start_transaction(offset);
        }

        if let Some(kind) = self.interrupt.dmc_dma_request.take() {
            self.dmc_dma = Some(DmcDmaState::from_kind(kind));
        }

        if self.process_dma_cycle_bus(ppu, &mut cartridge, controller, apu) {
            return;
        }

        let mut machine = self.cpu_stepfunc;
        self.internal_stat.step += 1;
        while let CpuStepStateEnum::Exit(s) = machine(self, ppu, &mut cartridge, controller, apu) {
            self.set_cpu_state(s);
            self.internal_stat.step = 1;
            machine = self.cpu_stepfunc;
        }
        self.sample_interrupt();
    }

    pub(crate) fn step_until_instruction_boundary(
        &mut self,
        ppu: &mut Ppu,
        cartridge: &mut dyn MapperCartridge,
        controller: &mut dyn Controller,
        apu: &mut Apu,
    ) -> u64 {
        let mut cycles = 0;
        loop {
            cycles += 1;
            self.step(ppu, cartridge, controller, apu);
            if self.is_instruction_boundary() {
                return cycles;
            }
        }
    }

    pub(crate) fn instruction_fast_path_max_cycles(
        &self,
        cartridge: &dyn MapperCartridge,
    ) -> Option<u64> {
        self.fast_path_plan(cartridge).map(|plan| plan.cycles)
    }

    fn fast_path_plan(&self, cartridge: &dyn MapperCartridge) -> Option<FastPathPlan> {
        if !self.is_instruction_boundary() || self.has_pending_dma() || self.has_pending_interrupt()
        {
            return None;
        }

        let opcode = self.internal_stat.get_opcode();
        let addressing = self.addressing_tables.get(opcode);
        let operation = self.opcode_tables.get(opcode);
        let pc = self.register.get_pc();

        if Self::operation_is_fast_path_branch(operation) {
            return self.fast_path_branch_plan(pc, operation, cartridge);
        }

        match addressing {
            CpuStatesEnum::Immediate => {
                if !Self::operation_uses_fast_path_read_operand(operation) {
                    return None;
                }
                let next_opcode_pc = pc.wrapping_add(1);
                self.peek_cpu_read(usize::from(pc), cartridge)?;
                self.peek_cpu_read(usize::from(next_opcode_pc), cartridge)?;
                Some(FastPathPlan {
                    cycles: 2,
                    kind: FastPathPlanKind::Immediate,
                })
            }
            CpuStatesEnum::Accumulator | CpuStatesEnum::Implied => {
                if !Self::operation_is_fast_path_implied_or_accumulator(operation) {
                    return None;
                }
                self.peek_cpu_read(usize::from(pc), cartridge)?;
                Some(FastPathPlan {
                    cycles: 2,
                    kind: FastPathPlanKind::ImpliedOrAccumulator,
                })
            }
            CpuStatesEnum::ZeroPage
            | CpuStatesEnum::ZeroPageX
            | CpuStatesEnum::ZeroPageY
            | CpuStatesEnum::Absolute
            | CpuStatesEnum::AbsoluteX
            | CpuStatesEnum::AbsoluteY => {
                let access = self.fast_path_memory_access(addressing, pc, cartridge)?;
                if Self::operation_uses_fast_path_read_operand(operation) {
                    self.peek_cpu_read(access.final_address, cartridge)?;
                    Some(FastPathPlan {
                        cycles: access.cycles,
                        kind: FastPathPlanKind::ReadMemory(access),
                    })
                } else if Self::operation_uses_fast_path_store_operand(operation)
                    && Self::cpu_write_is_fast_path_safe(access.final_address)
                {
                    Some(FastPathPlan {
                        cycles: access.cycles,
                        kind: FastPathPlanKind::StoreMemory(access),
                    })
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    pub(crate) fn step_fast_path_instruction(
        &mut self,
        ppu: &mut Ppu,
        cartridge: &mut dyn MapperCartridge,
        controller: &mut dyn Controller,
        apu: &mut Apu,
    ) -> Option<u64> {
        let plan = self.fast_path_plan(cartridge)?;
        let opcode = self.internal_stat.get_opcode();
        let operation = self.opcode_tables.get(opcode);
        let mut cartridge = mapper_cartridge_bus(cartridge);

        match plan.kind {
            FastPathPlanKind::Immediate => {
                self.cycles = self.cycles.wrapping_add(1);
                let address = self.register.get_pc();
                self.internal_stat.set_address(usize::from(address));
                let operand = self.memory.read_next(
                    &mut self.register,
                    ppu,
                    &mut cartridge,
                    controller,
                    apu,
                    &mut self.interrupt,
                );
                self.apply_fast_path_operation(operation, Some(operand))?;
                self.sample_interrupt();
                self.fetch_fast_path_next_opcode(ppu, &mut cartridge, controller, apu);
            }
            FastPathPlanKind::ImpliedOrAccumulator => {
                self.cycles = self.cycles.wrapping_add(1);
                let pc = usize::from(self.register.get_pc());
                let _ = self.memory.read(
                    pc,
                    ppu,
                    &mut cartridge,
                    controller,
                    apu,
                    &mut self.interrupt,
                );
                self.apply_fast_path_operation(operation, None)?;
                self.sample_interrupt();
                self.fetch_fast_path_next_opcode(ppu, &mut cartridge, controller, apu);
            }
            FastPathPlanKind::ReadMemory(access) => {
                self.execute_fast_path_addressing(access, ppu, &mut cartridge, controller, apu);
                self.cycles = self.cycles.wrapping_add(1);
                self.internal_stat.set_address(access.final_address);
                let operand = self.memory.read(
                    access.final_address,
                    ppu,
                    &mut cartridge,
                    controller,
                    apu,
                    &mut self.interrupt,
                );
                self.apply_fast_path_operation(operation, Some(operand))?;
                self.sample_interrupt();
                self.fetch_fast_path_next_opcode(ppu, &mut cartridge, controller, apu);
            }
            FastPathPlanKind::StoreMemory(access) => {
                self.execute_fast_path_addressing(access, ppu, &mut cartridge, controller, apu);
                self.cycles = self.cycles.wrapping_add(1);
                self.internal_stat.set_address(access.final_address);
                let value = self.fast_path_store_value(operation)?;
                self.memory.write(
                    access.final_address,
                    value,
                    ppu,
                    &mut cartridge,
                    controller,
                    apu,
                    &mut self.interrupt,
                );
                self.sample_interrupt();
                self.fetch_fast_path_next_opcode(ppu, &mut cartridge, controller, apu);
            }
            FastPathPlanKind::Branch(branch) => {
                self.execute_fast_path_branch(
                    branch,
                    operation,
                    ppu,
                    &mut cartridge,
                    controller,
                    apu,
                )?;
            }
        }

        Some(plan.cycles)
    }

    pub(crate) fn has_pending_dma(&self) -> bool {
        self.interrupt.oam_dma.is_some()
            || self.interrupt.dmc_dma_request.is_some()
            || self.dmc_dma.is_some()
            || self
                .oam_dma
                .as_ref()
                .is_some_and(OamDmaState::has_transaction)
    }

    fn is_instruction_boundary(&self) -> bool {
        self.internal_stat.get_state() == CpuStatesEnum::FetchOpCode
            && self.internal_stat.get_step() == 1
    }

    fn has_pending_interrupt(&self) -> bool {
        self.interrupt.nmi
            || self.interrupt.detected
            || self.interrupt.executing
            || !self.interrupt.irq_flag.is_empty()
    }

    fn fast_path_memory_access(
        &self,
        addressing: CpuStatesEnum,
        pc: u16,
        cartridge: &dyn MapperCartridge,
    ) -> Option<FastPathMemoryAccess> {
        let operand_low = self.peek_cpu_read(usize::from(pc), cartridge)?;
        let next_opcode_pc;
        let final_address;
        let dummy_read_address;
        let temp_address;
        let operand_bytes;
        let cycles;

        match addressing {
            CpuStatesEnum::ZeroPage => {
                next_opcode_pc = pc.wrapping_add(1);
                final_address = usize::from(operand_low);
                dummy_read_address = None;
                temp_address = None;
                operand_bytes = 1;
                cycles = 3;
            }
            CpuStatesEnum::ZeroPageX | CpuStatesEnum::ZeroPageY => {
                next_opcode_pc = pc.wrapping_add(1);
                let index = if addressing == CpuStatesEnum::ZeroPageX {
                    self.register.get_x()
                } else {
                    self.register.get_y()
                };
                final_address = usize::from(operand_low.wrapping_add(index));
                let dummy = (usize::from(next_opcode_pc) & 0xFF00) | usize::from(operand_low);
                self.peek_cpu_read(dummy, cartridge)?;
                dummy_read_address = Some(dummy);
                temp_address = None;
                operand_bytes = 1;
                cycles = 4;
            }
            CpuStatesEnum::Absolute => {
                let high_pc = pc.wrapping_add(1);
                let operand_high = self.peek_cpu_read(usize::from(high_pc), cartridge)?;
                next_opcode_pc = pc.wrapping_add(2);
                final_address = usize::from(operand_low) | (usize::from(operand_high) << 8);
                dummy_read_address = None;
                temp_address = None;
                operand_bytes = 2;
                cycles = 4;
            }
            CpuStatesEnum::AbsoluteX | CpuStatesEnum::AbsoluteY => {
                let high_pc = pc.wrapping_add(1);
                let operand_high = self.peek_cpu_read(usize::from(high_pc), cartridge)?;
                next_opcode_pc = pc.wrapping_add(2);
                let base = usize::from(operand_low) | (usize::from(operand_high) << 8);
                let index = if addressing == CpuStatesEnum::AbsoluteX {
                    usize::from(self.register.get_x())
                } else {
                    usize::from(self.register.get_y())
                };
                final_address = base.wrapping_add(index) & 0xFFFF;
                if page_crossed(base, final_address) {
                    let dummy = (base & 0xFF00) | (final_address & 0x00FF);
                    self.peek_cpu_read(dummy, cartridge)?;
                    dummy_read_address = Some(dummy);
                    cycles = 5;
                } else {
                    dummy_read_address = None;
                    cycles = 4;
                }
                temp_address = Some(base);
                operand_bytes = 2;
            }
            _ => return None,
        }

        self.peek_cpu_read(usize::from(next_opcode_pc), cartridge)?;
        Some(FastPathMemoryAccess {
            final_address,
            next_opcode_pc,
            operand_bytes,
            dummy_read_address,
            temp_address,
            cycles,
        })
    }

    fn fast_path_branch_plan(
        &self,
        pc: u16,
        operation: CpuStatesEnum,
        cartridge: &dyn MapperCartridge,
    ) -> Option<FastPathPlan> {
        let offset = self.peek_cpu_read(usize::from(pc), cartridge)?;
        let fallthrough = pc.wrapping_add(1);
        let target = fallthrough
            .wrapping_add(u16::from(offset))
            .wrapping_sub(if offset < 0x80 { 0 } else { 0x100 });
        let taken = Self::fast_path_branch_condition(operation, &self.register)?;
        if !taken {
            self.peek_cpu_read(usize::from(fallthrough), cartridge)?;
            return Some(FastPathPlan {
                cycles: 2,
                kind: FastPathPlanKind::Branch(FastPathBranch {
                    taken: false,
                    target,
                    fallthrough,
                    crossed: false,
                }),
            });
        }

        self.peek_cpu_read(usize::from(fallthrough), cartridge)?;
        let crossed = page_crossed(usize::from(target), usize::from(fallthrough));
        self.peek_cpu_read(usize::from(target), cartridge)?;
        Some(FastPathPlan {
            cycles: if crossed { 4 } else { 3 },
            kind: FastPathPlanKind::Branch(FastPathBranch {
                taken,
                target,
                fallthrough,
                crossed,
            }),
        })
    }

    fn execute_fast_path_addressing(
        &mut self,
        access: FastPathMemoryAccess,
        ppu: &mut Ppu,
        cartridge: &mut dyn Cartridge,
        controller: &mut dyn Controller,
        apu: &mut Apu,
    ) {
        for _ in 0..access.operand_bytes {
            self.cycles = self.cycles.wrapping_add(1);
            let _ = self.memory.read_next(
                &mut self.register,
                ppu,
                cartridge,
                controller,
                apu,
                &mut self.interrupt,
            );
            self.sample_interrupt();
        }

        if let Some(dummy_read_address) = access.dummy_read_address {
            self.cycles = self.cycles.wrapping_add(1);
            let _ = self.memory.read(
                dummy_read_address,
                ppu,
                cartridge,
                controller,
                apu,
                &mut self.interrupt,
            );
            self.sample_interrupt();
        }

        if let Some(temp_address) = access.temp_address {
            self.internal_stat.set_tempaddr(temp_address);
        }
        debug_assert_eq!(self.register.get_pc(), access.next_opcode_pc);
    }

    fn execute_fast_path_branch(
        &mut self,
        branch: FastPathBranch,
        operation: CpuStatesEnum,
        ppu: &mut Ppu,
        cartridge: &mut dyn Cartridge,
        controller: &mut dyn Controller,
        apu: &mut Apu,
    ) -> Option<()> {
        self.cycles = self.cycles.wrapping_add(1);
        let offset = self.memory.read_next(
            &mut self.register,
            ppu,
            cartridge,
            controller,
            apu,
            &mut self.interrupt,
        );
        let pc = self.register.get_pc();
        let target = pc
            .wrapping_add(u16::from(offset))
            .wrapping_sub(if offset < 0x80 { 0 } else { 0x100 });
        debug_assert_eq!(target, branch.target);
        self.internal_stat.set_address(usize::from(target));
        self.sample_interrupt();

        self.cycles = self.cycles.wrapping_add(1);
        self.internal_stat.set_crossed(true);
        self.internal_stat.set_interrupt(self.interrupt.executing);
        if !Self::fast_path_branch_condition(operation, &self.register)? {
            debug_assert!(!branch.taken);
            self.fetch_fast_path_next_opcode_without_cycle_sample(ppu, cartridge, controller, apu);
            return Some(());
        }
        debug_assert!(branch.taken);
        let _ = self.memory.read(
            usize::from(branch.fallthrough),
            ppu,
            cartridge,
            controller,
            apu,
            &mut self.interrupt,
        );
        self.internal_stat.set_crossed(branch.crossed);
        self.sample_interrupt();

        if branch.crossed {
            self.cycles = self.cycles.wrapping_add(1);
            let _ = self.memory.read(
                usize::from(branch.fallthrough),
                ppu,
                cartridge,
                controller,
                apu,
                &mut self.interrupt,
            );
            self.register.set_pc(branch.target);
            self.sample_interrupt();
            self.fetch_fast_path_next_opcode(ppu, cartridge, controller, apu);
        } else {
            self.cycles = self.cycles.wrapping_add(1);
            self.register.set_pc(branch.target);
            if !self.internal_stat.get_interrupt() {
                self.interrupt.executing = false;
            }
            self.fetch_fast_path_next_opcode_without_cycle_sample(ppu, cartridge, controller, apu);
        }

        Some(())
    }

    fn fetch_fast_path_next_opcode(
        &mut self,
        ppu: &mut Ppu,
        cartridge: &mut dyn Cartridge,
        controller: &mut dyn Controller,
        apu: &mut Apu,
    ) {
        self.cycles = self.cycles.wrapping_add(1);
        self.fetch_fast_path_next_opcode_without_cycle_sample(ppu, cartridge, controller, apu);
    }

    fn fetch_fast_path_next_opcode_without_cycle_sample(
        &mut self,
        ppu: &mut Ppu,
        cartridge: &mut dyn Cartridge,
        controller: &mut dyn Controller,
        apu: &mut Apu,
    ) {
        let next_opcode = self.memory.read_next(
            &mut self.register,
            ppu,
            cartridge,
            controller,
            apu,
            &mut self.interrupt,
        );
        self.internal_stat.set_opcode(usize::from(next_opcode));
        self.set_cpu_state(CpuStatesEnum::FetchOpCode);
        self.internal_stat.set_step(1);
        self.sample_interrupt();
    }

    fn peek_cpu_read(&self, address: usize, cartridge: &dyn MapperCartridge) -> Option<u8> {
        if !Self::cpu_read_is_fast_path_safe(address, cartridge) {
            return None;
        }

        match address {
            0x0000..=0x1FFF => self.memory.peek_work_ram(address),
            0x6000..=0xFFFF => {
                let result = cartridge.read(address);
                (result.mask == 0xFF).then_some(result.data)
            }
            _ => None,
        }
    }

    fn cpu_read_is_fast_path_safe(address: usize, cartridge: &dyn MapperCartridge) -> bool {
        match address {
            0x0000..=0x1FFF => true,
            0x6000..=0xFFFF => !cartridge.cpu_read_has_side_effect(address),
            _ => false,
        }
    }

    fn cpu_write_is_fast_path_safe(address: usize) -> bool {
        address <= 0x1FFF
    }

    fn operation_uses_fast_path_read_operand(state: CpuStatesEnum) -> bool {
        matches!(
            state,
            CpuStatesEnum::Adc
                | CpuStatesEnum::And
                | CpuStatesEnum::Bit
                | CpuStatesEnum::Cmp
                | CpuStatesEnum::Cpx
                | CpuStatesEnum::Cpy
                | CpuStatesEnum::Eor
                | CpuStatesEnum::Lda
                | CpuStatesEnum::Ldx
                | CpuStatesEnum::Ldy
                | CpuStatesEnum::Ora
                | CpuStatesEnum::Sbc
        )
    }

    fn operation_uses_fast_path_store_operand(state: CpuStatesEnum) -> bool {
        matches!(
            state,
            CpuStatesEnum::Sta | CpuStatesEnum::Stx | CpuStatesEnum::Sty
        )
    }

    fn operation_is_fast_path_implied_or_accumulator(state: CpuStatesEnum) -> bool {
        matches!(
            state,
            CpuStatesEnum::AslAcc
                | CpuStatesEnum::Clc
                | CpuStatesEnum::Cld
                | CpuStatesEnum::Cli
                | CpuStatesEnum::Clv
                | CpuStatesEnum::Dex
                | CpuStatesEnum::Dey
                | CpuStatesEnum::Inx
                | CpuStatesEnum::Iny
                | CpuStatesEnum::LsrAcc
                | CpuStatesEnum::Nop
                | CpuStatesEnum::RolAcc
                | CpuStatesEnum::RorAcc
                | CpuStatesEnum::Sec
                | CpuStatesEnum::Sed
                | CpuStatesEnum::Sei
                | CpuStatesEnum::Tax
                | CpuStatesEnum::Tay
                | CpuStatesEnum::Tsx
                | CpuStatesEnum::Txa
                | CpuStatesEnum::Txs
                | CpuStatesEnum::Tya
        )
    }

    fn operation_is_fast_path_branch(state: CpuStatesEnum) -> bool {
        matches!(
            state,
            CpuStatesEnum::Bcc
                | CpuStatesEnum::Bcs
                | CpuStatesEnum::Beq
                | CpuStatesEnum::Bmi
                | CpuStatesEnum::Bne
                | CpuStatesEnum::Bpl
                | CpuStatesEnum::Bvc
                | CpuStatesEnum::Bvs
        )
    }

    fn fast_path_branch_condition(state: CpuStatesEnum, register: &Register) -> Option<bool> {
        Some(match state {
            CpuStatesEnum::Bcc => !register.get_c(),
            CpuStatesEnum::Bcs => register.get_c(),
            CpuStatesEnum::Beq => register.get_z(),
            CpuStatesEnum::Bmi => register.get_n(),
            CpuStatesEnum::Bne => !register.get_z(),
            CpuStatesEnum::Bpl => !register.get_n(),
            CpuStatesEnum::Bvc => !register.get_v(),
            CpuStatesEnum::Bvs => register.get_v(),
            _ => return None,
        })
    }

    fn fast_path_store_value(&self, operation: CpuStatesEnum) -> Option<u8> {
        Some(match operation {
            CpuStatesEnum::Sta => self.register.get_a(),
            CpuStatesEnum::Stx => self.register.get_x(),
            CpuStatesEnum::Sty => self.register.get_y(),
            _ => return None,
        })
    }

    fn apply_fast_path_operation(
        &mut self,
        operation: CpuStatesEnum,
        operand: Option<u8>,
    ) -> Option<()> {
        match operation {
            CpuStatesEnum::Adc => {
                let a = u16::from(self.register.get_a());
                let b = u16::from(operand?);
                let c = if self.register.get_c() { 1 } else { 0 };
                let result = a + b + c;
                self.register.set_c(result > 0xFF);
                self.register
                    .set_v((a ^ b) & 0x80 == 0 && (a ^ result) & 0x80 != 0);
                self.set_accumulator_result((result & 0xFF) as u8);
            }
            CpuStatesEnum::And => self.set_accumulator_result(self.register.get_a() & operand?),
            CpuStatesEnum::AslAcc => {
                let value = self.register.get_a();
                self.register.set_c(value & 0x80 != 0);
                self.set_accumulator_result(value << 1);
            }
            CpuStatesEnum::Clc => self.register.set_c(false),
            CpuStatesEnum::Cld => self.register.set_d(false),
            CpuStatesEnum::Cli => self.register.set_i(false),
            CpuStatesEnum::Clv => self.register.set_v(false),
            CpuStatesEnum::Bit => {
                let data = operand?;
                self.register.set_v(data & 0x40 != 0);
                self.register.set_z_from_value(data & self.register.get_a());
                self.register.set_n_from_value(data);
            }
            CpuStatesEnum::Cmp => self.compare_fast_path(self.register.get_a(), operand?),
            CpuStatesEnum::Cpx => self.compare_fast_path(self.register.get_x(), operand?),
            CpuStatesEnum::Cpy => self.compare_fast_path(self.register.get_y(), operand?),
            CpuStatesEnum::Dex => {
                let value = self.register.get_x().wrapping_sub(1);
                self.register.set_nz_from_value(value);
                self.register.set_x(value);
            }
            CpuStatesEnum::Dey => {
                let value = self.register.get_y().wrapping_sub(1);
                self.register.set_nz_from_value(value);
                self.register.set_y(value);
            }
            CpuStatesEnum::Eor => self.set_accumulator_result(self.register.get_a() ^ operand?),
            CpuStatesEnum::Inx => {
                let value = self.register.get_x().wrapping_add(1);
                self.register.set_nz_from_value(value);
                self.register.set_x(value);
            }
            CpuStatesEnum::Iny => {
                let value = self.register.get_y().wrapping_add(1);
                self.register.set_nz_from_value(value);
                self.register.set_y(value);
            }
            CpuStatesEnum::Lda => self.load_a_fast_path(operand?),
            CpuStatesEnum::Ldx => self.load_x_fast_path(operand?),
            CpuStatesEnum::Ldy => self.load_y_fast_path(operand?),
            CpuStatesEnum::LsrAcc => {
                let value = self.register.get_a();
                self.register.set_c(value & 0x01 != 0);
                self.set_accumulator_result(value >> 1);
            }
            CpuStatesEnum::Nop => {}
            CpuStatesEnum::Ora => self.set_accumulator_result(self.register.get_a() | operand?),
            CpuStatesEnum::RolAcc => {
                let value = self.register.get_a();
                let carry = if self.register.get_c() { 1 } else { 0 };
                self.register.set_c(value & 0x80 != 0);
                self.set_accumulator_result(value << 1 | carry);
            }
            CpuStatesEnum::RorAcc => {
                let value = self.register.get_a();
                let carry = if self.register.get_c() { 0x80 } else { 0 };
                self.register.set_c(value & 0x01 != 0);
                self.set_accumulator_result(value >> 1 | carry);
            }
            CpuStatesEnum::Sbc => {
                let a = u16::from(self.register.get_a());
                let b = u16::from(operand?);
                let c = if self.register.get_c() { 0 } else { 1 };
                let result = a.wrapping_sub(b).wrapping_sub(c);
                self.register.set_c(result <= 0xFF);
                self.register
                    .set_v((a ^ b) & 0x80 != 0 && (a ^ result) & 0x80 != 0);
                self.set_accumulator_result((result & 0xFF) as u8);
            }
            CpuStatesEnum::Sec => self.register.set_c(true),
            CpuStatesEnum::Sed => self.register.set_d(true),
            CpuStatesEnum::Sei => self.register.set_i(true),
            CpuStatesEnum::Tax => self.load_x_fast_path(self.register.get_a()),
            CpuStatesEnum::Tay => self.load_y_fast_path(self.register.get_a()),
            CpuStatesEnum::Tsx => self.load_x_fast_path(self.register.get_sp()),
            CpuStatesEnum::Txa => self.load_a_fast_path(self.register.get_x()),
            CpuStatesEnum::Txs => self.register.set_sp(self.register.get_x()),
            CpuStatesEnum::Tya => self.load_a_fast_path(self.register.get_y()),
            _ => return None,
        }
        Some(())
    }

    fn sample_interrupt(&mut self) {
        self.interrupt.executing = self.interrupt.detected;
        self.interrupt.detected = self.interrupt.nmi
            || (!((self.interrupt.irq_flag & self.interrupt.irq_mask).is_empty())
                && !self.register.get_i());
    }

    fn set_accumulator_result(&mut self, value: u8) {
        self.register.set_nz_from_value(value);
        self.register.set_a(value);
    }

    fn compare_fast_path(&mut self, left: u8, right: u8) {
        self.register.set_nz_from_value(left.wrapping_sub(right));
        self.register.set_c(left >= right);
    }

    fn load_a_fast_path(&mut self, value: u8) {
        self.register.set_nz_from_value(value);
        self.register.set_a(value);
    }

    fn load_x_fast_path(&mut self, value: u8) {
        self.register.set_nz_from_value(value);
        self.register.set_x(value);
    }

    fn load_y_fast_path(&mut self, value: u8) {
        self.register.set_nz_from_value(value);
        self.register.set_y(value);
    }

    pub(crate) fn interrupt_mut(&mut self) -> &mut Interrupt {
        &mut self.interrupt
    }

    pub(crate) fn interrupt_ref(&self) -> &Interrupt {
        &self.interrupt
    }

    pub(crate) fn validate_runtime_state(&self) -> Result<(), PersistenceError> {
        self.internal_stat.validate()
    }

    #[cfg(test)]
    pub(crate) fn set_internal_opcode_for_test(&mut self, value: usize) {
        self.internal_stat.set_opcode(value);
    }

    fn process_dma_cycle(
        &mut self,
        ppu: &mut Ppu,
        cartridge: &mut dyn MapperCartridge,
        controller: &mut dyn Controller,
        apu: &mut Apu,
    ) -> bool {
        let mut cartridge = mapper_cartridge_bus(cartridge);
        self.process_dma_cycle_bus(ppu, &mut cartridge, controller, apu)
    }

    fn process_dma_cycle_bus(
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

impl Clone for Core {
    fn clone(&self) -> Self {
        Self {
            opcode_tables: Opcodes::new(),
            addressing_tables: AddressingModeLut::new(),
            memory: self.memory.clone(),
            register: self.register.clone(),
            internal_stat: self.internal_stat.clone(),
            interrupt: self.interrupt,
            cycles: self.cycles,
            oam_dma: self.oam_dma.clone(),
            dmc_dma: self.dmc_dma,
            cpu_stepfunc: cpu_stepfunc(self.internal_stat.state),
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
    use super::{
        CPU_STEPFUNCS, Core, CpuStatesEnum, CpuStepStateFunc, DmcDmaKind, DmcDmaPhase, DmcDmaState,
    };
    use crate::controller::Controller;
    use crate::{Apu, OpenBusReadResult, Ppu};
    use strum::IntoEnumIterator;

    macro_rules! cpu_stepfunc_pair_array {
        ($(($state:expr, $func:expr)),+ $(,)?) => {
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

    #[derive(Default)]
    struct TestController;

    impl Controller for TestController {
        fn read(&mut self, _address: usize) -> OpenBusReadResult {
            OpenBusReadResult::new(0, 0)
        }

        fn write(&mut self, _value: u8) {}
    }

    fn dmc_dma_stall_cycles(cpu: &mut Core) -> usize {
        let mut ppu = Ppu::new();
        let mut cartridge = super::super::nrom_test_cartridge();
        let mut controller = TestController;
        let mut apu = Apu::new(cpu.interrupt_mut());
        let mut stalled_cycles = 0;

        while cpu.dmc_dma.is_some() {
            cpu.cycles += 1;
            if cpu.process_dma_cycle(&mut ppu, cartridge.as_mut(), &mut controller, &mut apu) {
                stalled_cycles += 1;
            }
        }

        stalled_cycles
    }

    fn prepare_read_cycle_cpu() -> Core {
        let mut cpu = Core::new();
        cpu.set_cpu_state(CpuStatesEnum::FetchOpCode);
        cpu.internal_stat.step = 1;
        cpu
    }

    fn set_write_cycle(cpu: &mut Core) {
        cpu.set_cpu_state(CpuStatesEnum::Pha);
        cpu.internal_stat.step = 1;
    }

    #[test]
    fn load_dmc_dma_stalls_cpu_for_three_cycles_in_common_case() {
        let mut cpu = prepare_read_cycle_cpu();
        cpu.cycles = 0;
        cpu.dmc_dma = Some(DmcDmaState::from_kind(DmcDmaKind::Load));

        assert_eq!(dmc_dma_stall_cycles(&mut cpu), 3);
        assert!(cpu.dmc_dma.is_none());
    }

    #[test]
    fn reload_dmc_dma_stalls_cpu_for_four_cycles_in_common_case() {
        let mut cpu = prepare_read_cycle_cpu();
        cpu.cycles = 1;
        cpu.dmc_dma = Some(DmcDmaState::from_kind(DmcDmaKind::Reload));

        assert_eq!(dmc_dma_stall_cycles(&mut cpu), 4);
        assert!(cpu.dmc_dma.is_none());
    }

    #[test]
    fn load_dmc_dma_reads_pending_sample_byte() {
        let mut cpu = prepare_read_cycle_cpu();
        cpu.cycles = 0;
        let mut ppu = Ppu::new();
        let mut cartridge = super::super::nrom_test_cartridge();
        let mut controller = TestController;
        let mut apu = Apu::new(cpu.interrupt_mut());

        apu.write_register(0x4010, 0x00, cpu.interrupt_mut());
        apu.write_register(0x4012, 0x00, cpu.interrupt_mut());
        apu.write_register(0x4013, 0x00, cpu.interrupt_mut());
        apu.write_register(0x4015, 0x10, cpu.interrupt_mut());
        cpu.dmc_dma = Some(DmcDmaState::from_kind(DmcDmaKind::Load));

        assert_eq!(
            apu.read_register(0x4015, cpu.interrupt_mut()).data & 0x10,
            0x10
        );

        while cpu.dmc_dma.is_some() {
            cpu.cycles += 1;
            cpu.process_dma_cycle(&mut ppu, cartridge.as_mut(), &mut controller, &mut apu);
        }

        assert_eq!(
            apu.read_register(0x4015, cpu.interrupt_mut()).data & 0x10,
            0x00
        );
    }

    #[test]
    fn load_dmc_dma_adds_alignment_cycle_after_write_delayed_halt() {
        let mut cpu = prepare_read_cycle_cpu();
        cpu.cycles = 0;
        cpu.dmc_dma = Some(DmcDmaState {
            delay: 0,
            halt_on_get_cycle: true,
            halted_on_get_cycle: false,
            attempted_halt: false,
            phase: DmcDmaPhase::WaitForHalt,
        });

        let mut ppu = Ppu::new();
        let mut cartridge = super::super::nrom_test_cartridge();
        let mut controller = TestController;
        let mut apu = Apu::new(cpu.interrupt_mut());

        set_write_cycle(&mut cpu);
        cpu.cycles += 1;
        assert!(!cpu.process_dma_cycle(&mut ppu, cartridge.as_mut(), &mut controller, &mut apu));

        cpu.set_cpu_state(CpuStatesEnum::FetchOpCode);
        cpu.internal_stat.step = 1;

        let mut stalled = 0;
        while cpu.dmc_dma.is_some() {
            cpu.cycles += 1;
            if cpu.process_dma_cycle(&mut ppu, cartridge.as_mut(), &mut controller, &mut apu) {
                stalled += 1;
            }
        }

        assert_eq!(stalled, 4);
    }

    #[test]
    fn reload_dmc_dma_skips_alignment_when_write_delay_flips_parity() {
        let mut cpu = prepare_read_cycle_cpu();
        cpu.cycles = 1;
        cpu.dmc_dma = Some(DmcDmaState {
            delay: 0,
            halt_on_get_cycle: false,
            halted_on_get_cycle: false,
            attempted_halt: false,
            phase: DmcDmaPhase::WaitForHalt,
        });

        let mut ppu = Ppu::new();
        let mut cartridge = super::super::nrom_test_cartridge();
        let mut controller = TestController;
        let mut apu = Apu::new(cpu.interrupt_mut());

        set_write_cycle(&mut cpu);
        cpu.cycles += 1;
        assert!(!cpu.process_dma_cycle(&mut ppu, cartridge.as_mut(), &mut controller, &mut apu));

        cpu.set_cpu_state(CpuStatesEnum::FetchOpCode);
        cpu.internal_stat.step = 1;

        let mut stalled = 0;
        while cpu.dmc_dma.is_some() {
            cpu.cycles += 1;
            if cpu.process_dma_cycle(&mut ppu, cartridge.as_mut(), &mut controller, &mut apu) {
                stalled += 1;
            }
        }

        assert_eq!(stalled, 3);
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
