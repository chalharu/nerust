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
    registers: CpuRegisters,
    phase: Phase,
    ime_delayed: bool,
    opcode: u8,
    operands: [u8; 2],
    operand_count: u8,
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

    pub(crate) fn phase(&self) -> Phase {
        self.phase
    }
    pub(crate) fn set_phase(&mut self, p: Phase) {
        self.phase = p;
    }
    pub(crate) fn ime_delayed(&self) -> bool {
        self.ime_delayed
    }
    pub(crate) fn set_ime_delayed(&mut self, v: bool) {
        self.ime_delayed = v;
    }
    pub(crate) fn registers(&self) -> &CpuRegisters {
        &self.registers
    }
    pub(crate) fn registers_mut(&mut self) -> &mut CpuRegisters {
        &mut self.registers
    }
    pub(crate) fn opcode(&self) -> u8 {
        self.opcode
    }
    pub(crate) fn set_opcode(&mut self, v: u8) {
        self.opcode = v;
    }
    pub(crate) fn operand(&self, idx: usize) -> u8 {
        self.operands[idx]
    }
    pub(crate) fn set_operand(&mut self, idx: usize, v: u8) {
        self.operands[idx] = v;
    }
    pub(crate) fn operand_count(&self) -> u8 {
        self.operand_count
    }
    pub(crate) fn set_operand_count(&mut self, v: u8) {
        self.operand_count = v;
    }

    pub(crate) fn pc_read(&mut self, bus: &mut GbcMemoryBus) -> u8 {
        let b = bus.read(self.registers().pc());
        let pc = self.registers().pc();
        self.registers_mut().set_pc(pc.wrapping_add(1));
        b
    }
}

impl Default for Lr35902Cpu {
    fn default() -> Self {
        Self::new()
    }
}
