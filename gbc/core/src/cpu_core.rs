use crate::cpu_registers::CpuRegisters;
use crate::memory::GbcMemoryBus;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum StepResult {
    Continue,
    Exit,
}

pub(crate) type HandlerFn = fn(&mut Lr35902Cpu, &mut GbcMemoryBus, u8) -> StepResult;

#[derive(Debug, Clone, Copy)]
pub(crate) enum Phase {
    FetchOpcode,
    ExecuteOpcode { handler: HandlerFn, step: u8 },
}

pub struct Lr35902Cpu {
    pub registers: CpuRegisters,
    pub(crate) phase: Phase,
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
