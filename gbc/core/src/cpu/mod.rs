//! LR35902 CPU — per-M-cycle state machine.
//!
//! Pattern follows the NES CPU (`nes/core/src/cpu/`):
//! - `step()` advances exactly one M-cycle
//! - Internal state (`CpuState` + `m_cycle`) tracks multi-cycle instructions
//! - Handlers in `handlers.rs` decompose each opcode into M-cycle steps
//! - Handlers return `Continue` (stay) or `Exit` (transition to FetchOpcode)

pub mod handlers;
pub mod registers;

use crate::cpu::registers::CpuRegisters;
use crate::interrupt::InterruptKind;
use crate::memory::GbcMemoryBus;

/// Result returned by opcode handlers each M-cycle.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum StepResult {
    Continue,
    Exit,
}

/// Handler function type: takes CPU + bus + current M-cycle step (1-based),
/// returns Continue or Exit.
type HandlerFn = fn(&mut Lr35902Cpu, &mut GbcMemoryBus, u8) -> StepResult;

/// Lookup table: opcode → handler function. Lazy-initialized.
static HANDLER_TABLE: std::sync::LazyLock<[HandlerFn; 256]> =
    std::sync::LazyLock::new(|| handlers::build_table());

/// Phases of the CPU state machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    FetchOpcode,
    ExecuteOpcode {
        handler: HandlerFn,
        /// M-cycle counter within the current instruction (1-based).
        /// Incremented each M-cycle. Resets to 1 on new instruction.
        m_cycle: u8,
    },
}

pub struct Lr35902Cpu {
    pub registers: CpuRegisters,
    phase: Phase,
    ime_delayed: bool,
    /// Pending opcode (set during FetchOpcode step 1).
    opcode: u8,
    /// Operand bytes fetched during instruction execution.
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

                // M1: read opcode byte
                let opcode = if bus.is_dma_active() {
                    bus.read_dma(self.registers.pc).unwrap_or(0xFF)
                } else {
                    bus.read(self.registers.pc)
                };
                self.registers.pc = self.registers.pc.wrapping_add(1);
                self.opcode = opcode;
                self.operands = [0; 2];
                self.operand_count = 0;

                let handler = HANDLER_TABLE[opcode as usize];

                // Try to execute step 1 immediately. If it exits, the
                // instruction completes in 1 M-cycle and we stay in FetchOpcode.
                match handler(self, bus, 1) {
                    StepResult::Exit => {
                        // Single M-cycle instruction: done.
                        self.phase = Phase::FetchOpcode;
                    }
                    StepResult::Continue => {
                        self.phase = Phase::ExecuteOpcode {
                            handler,
                            m_cycle: 1,
                        };
                    }
                }
            }
            Phase::ExecuteOpcode {
                handler,
                mut m_cycle,
            } => {
                m_cycle += 1;
                match handler(self, bus, m_cycle) {
                    StepResult::Exit => {
                        self.phase = Phase::FetchOpcode;
                    }
                    StepResult::Continue => {
                        self.phase = Phase::ExecuteOpcode { handler, m_cycle };
                    }
                }
            }
        }
    }

    // ── helpers for opcode handlers ───────────────────────

    /// Read next byte from PC and advance PC.
    pub(crate) fn fetch_pc_byte(&mut self, bus: &mut GbcMemoryBus) -> u8 {
        let b = if bus.is_dma_active() {
            bus.read_dma(self.registers.pc).unwrap_or(0xFF)
        } else {
            bus.read(self.registers.pc)
        };
        self.registers.pc = self.registers.pc.wrapping_add(1);
        b
    }

    /// Push PC to stack and jump to interrupt vector.
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
