//! LR35902 CPU — per-M-cycle state machine.
//!
//! Each `step()` call executes exactly **one M-cycle** (= 4 T-cycles).
//! Multi-M-cycle instructions are decomposed via `CpuStepState::exec()`.

use std::sync::LazyLock;

use crate::cpu_opcodes::HandlerFn;
use crate::cpu_registers::CpuRegisters;
use crate::interrupt::InterruptKind;
use crate::memory::GbcMemoryBus;

/// Returned by each M-cycle step of an instruction.
pub(crate) enum StepResult {
    Continue,
    Exit,
}

static TABLE: LazyLock<[HandlerFn; 256]> = LazyLock::new(|| crate::cpu_opcodes::handler_table());

/// Phases of the CPU state machine.
pub(crate) enum Phase {
    FetchOpcode,
    ExecuteOpcode { handler: HandlerFn, step: u8 },
}

pub struct Lr35902Cpu {
    pub registers: CpuRegisters,
    pub(crate) phase: Phase,
    pub(crate) ime_delayed: bool,
    /// Fetched opcode byte.
    pub(crate) opcode: u8,
    /// Operand bytes fetched during M2-Mn cycles.
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

    /// Step one M-cycle (= 4 T-cycles).
    pub fn step(&mut self, bus: &mut GbcMemoryBus) {
        if self.ime_delayed {
            bus.set_ime(true);
            self.ime_delayed = false;
        }
        if bus.is_halted_or_stopped() {
            return;
        }

        match self.phase {
            Phase::FetchOpcode => {
                self.check_interrupts(bus);
                let op = if bus.is_dma_active() {
                    bus.read_dma(self.registers.pc).unwrap_or(0xFF)
                } else {
                    bus.read(self.registers.pc)
                };
                self.registers.pc = self.registers.pc.wrapping_add(1);
                self.opcode = op;
                self.operands = [0; 2];
                self.operand_count = 0;

                let h = TABLE[op as usize];
                match h(self, bus, 1) {
                    StepResult::Exit => self.phase = Phase::FetchOpcode,
                    StepResult::Continue => {
                        self.phase = Phase::ExecuteOpcode {
                            handler: h,
                            step: 1,
                        }
                    }
                }
            }
            Phase::ExecuteOpcode { handler, step } => {
                let next = step + 1;
                match handler(self, bus, next) {
                    StepResult::Exit => self.phase = Phase::FetchOpcode,
                    StepResult::Continue => {
                        self.phase = Phase::ExecuteOpcode {
                            handler,
                            step: next,
                        }
                    }
                }
            }
        }
    }

    /// Read a byte from PC and advance PC.
    pub(crate) fn pc_read(&mut self, bus: &mut GbcMemoryBus) -> u8 {
        let b = if bus.is_dma_active() {
            bus.read_dma(self.registers.pc).unwrap_or(0xFF)
        } else {
            bus.read(self.registers.pc)
        };
        self.registers.pc = self.registers.pc.wrapping_add(1);
        b
    }

    fn check_interrupts(&mut self, bus: &mut GbcMemoryBus) {
        if bus.ime_enabled()
            && let Some(kind) = bus.acknowledge_interrupt()
        {
            self.dispatch_interrupt(kind, bus);
        }
    }

    fn dispatch_interrupt(&mut self, kind: InterruptKind, bus: &mut GbcMemoryBus) {
        self.registers.sp = self.registers.sp.wrapping_sub(1);
        bus.write(self.registers.sp, (self.registers.pc >> 8) as u8);
        self.registers.sp = self.registers.sp.wrapping_sub(1);
        bus.write(self.registers.sp, self.registers.pc as u8);
        self.registers.pc = kind.vector();
    }
}

impl Default for Lr35902Cpu {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests;
