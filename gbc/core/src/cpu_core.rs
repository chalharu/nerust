//! LR35902 CPU core types shared between cpu and cpu_opcodes.

use crate::cpu_registers::CpuRegisters;
use crate::memory::GbcMemoryBus;

/// Returned by each handler invocation.
pub(crate) enum StepResult {
    Continue,
    Exit,
}

/// Handler function pointer type.
/// Called at T3 (bus phase) and T4 (internal phase) of each M-cycle.
/// The handler checks `core.t_cycle` to distinguish phases:
///   t_cycle == 2 (T3): perform bus access, return Continue
///   t_cycle == 3 (T4): perform internal ops, return Continue or Exit
///   step == 1, T4: decode fetch, return Exit (1-cycle) or Continue (multi-cycle)
pub(crate) type HandlerFn = fn(&mut Lr35902Cpu, &mut GbcMemoryBus, u8) -> StepResult;

/// Phases of the CPU state machine.
pub(crate) enum Phase {
    FetchOpcode,
    ExecuteOpcode { handler: HandlerFn, step: u8 },
}

pub struct Lr35902Cpu {
    pub registers: CpuRegisters,
    pub(crate) phase: Phase,
    pub(crate) t_cycle: u8, // 0-3, current T-cycle within M-cycle
    pub(crate) ime_delayed: bool,
    pub(crate) opcode: u8,
    pub(crate) operands: [u8; 2],
    pub(crate) operand_count: u8,
}

impl Lr35902Cpu {
    pub fn new() -> Self {
        Self {
            registers: CpuRegisters::new(),
            phase: Phase::FetchOpcode,
            t_cycle: 0,
            ime_delayed: false,
            opcode: 0,
            operands: [0; 2],
            operand_count: 0,
        }
    }

    pub(crate) fn pc_read(&mut self, bus: &mut GbcMemoryBus) -> u8 {
        let b = if bus.is_dma_active() {
            bus.read_dma(self.registers.pc).unwrap_or(0xFF)
        } else {
            bus.read(self.registers.pc)
        };
        self.registers.pc = self.registers.pc.wrapping_add(1);
        b
    }
}

impl Default for Lr35902Cpu {
    fn default() -> Self {
        Self::new()
    }
}
