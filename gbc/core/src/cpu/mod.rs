//! LR35902 CPU — per-T-cycle state machine.
//!
//! Each `step_t_cycle()` call advances exactly **one T-cycle** (= 1/4 M-cycle).
//! Bus reads/writes happen at T3 of each M-cycle, matching real hardware.

use std::sync::LazyLock;

use crate::cpu_core::{HandlerFn, Lr35902Cpu, Phase, StepResult};
use crate::interrupt::InterruptKind;
use crate::memory::GbcMemoryBus;

static TABLE: LazyLock<[HandlerFn; 256]> = LazyLock::new(|| crate::cpu_opcodes::handler_table());

impl Lr35902Cpu {
    /// Step one T-cycle (= 1/4 of an M-cycle).
    /// Bus reads/writes occur at T3; internal operations at T4.
    pub fn step_t_cycle(&mut self, bus: &mut GbcMemoryBus) {
        bus.step_devices(1);

        if self.ime_delayed {
            bus.set_ime(true);
            self.ime_delayed = false;
        }
        if bus.is_halted_or_stopped() {
            return;
        }

        match self.t_cycle {
            2 => self.exec_t3(bus),
            3 => self.exec_t4(bus),
            _ => {}
        }

        self.t_cycle = (self.t_cycle + 1) & 3;
    }

    /// T3: bus access phase
    fn exec_t3(&mut self, bus: &mut GbcMemoryBus) {
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
            }
            Phase::ExecuteOpcode { handler, step } => {
                handler(self, bus, step);
            }
        }
    }

    /// T4: internal phase — decode fetch results, update registers,
    /// advance or complete the instruction
    fn exec_t4(&mut self, bus: &mut GbcMemoryBus) {
        match self.phase {
            Phase::FetchOpcode => {
                let h = TABLE[self.opcode as usize];
                // step=0 signals "fetch decode" to the handler
                match h(self, bus, 0) {
                    StepResult::Exit => {}
                    StepResult::Continue => {
                        self.phase = Phase::ExecuteOpcode {
                            handler: h,
                            step: 1,
                        }
                    }
                }
            }
            Phase::ExecuteOpcode { handler, step } => match handler(self, bus, step) {
                StepResult::Exit => self.phase = Phase::FetchOpcode,
                StepResult::Continue => {
                    self.phase = Phase::ExecuteOpcode {
                        handler,
                        step: step + 1,
                    }
                }
            },
        }
    }

    fn check_interrupts(&mut self, bus: &mut GbcMemoryBus) {
        if bus.ime_enabled()
            && let Some(kind) = bus.acknowledge_interrupt()
        {
            self.dispatch_interrupt(kind, bus);
        }
    }

    fn dispatch_interrupt(&mut self, kind: InterruptKind, bus: &mut GbcMemoryBus) {
        // TODO: This skips the usual per-T-cycle state machine for the
        // push-and-jump sequence. Each bus.write should take a full M-cycle.
        self.registers.sp = self.registers.sp.wrapping_sub(1);
        bus.write(self.registers.sp, (self.registers.pc >> 8) as u8);
        self.registers.sp = self.registers.sp.wrapping_sub(1);
        bus.write(self.registers.sp, self.registers.pc as u8);
        self.registers.pc = kind.vector();
    }
}

#[cfg(test)]
mod opcode_tests;
