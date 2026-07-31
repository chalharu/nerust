use std::sync::LazyLock;

use crate::cpu_core::{HandlerFn, Lr35902Cpu, Phase, StepResult, dispatch_interrupt};
use crate::memory::GbcMemoryBus;

static TABLE: LazyLock<[HandlerFn; 256]> = LazyLock::new(|| crate::cpu_opcodes::handler_table());

impl Lr35902Cpu {
    /// Step one M-cycle (no device advancement — caller must call step_devices).
    pub fn step(&mut self, bus: &mut GbcMemoryBus) {
        if self.ime_delayed() {
            bus.set_ime(true);
            self.set_ime_delayed(false);
        }
        if bus.is_halted_or_stopped() {
            self.check_interrupts(bus);
            if bus.is_halted_or_stopped() {
                return;
            }
        }

        match self.phase() {
            Phase::FetchOpcode => {
                self.check_interrupts(bus);
                // Interrupt dispatch takes 5 M-cycles on CGB D
                // (= 20 T-cycles of PPU advancement). The FetchOpcode
                // M-cycle that detects the interrupt is consumed as
                // the first dispatch M-cycle; subsequent M-cycles are
                // handled by the InterruptDispatch phase.
                if matches!(self.phase(), Phase::InterruptDispatch { .. }) {
                    return;
                }
                let op = bus.read(self.registers().pc());
                // HALT bug: when HALT is executed with IME=0 and a pending
                // interrupt, the CPU immediately wakes (doesn't halt), but
                // PC is not incremented during the next opcode fetch. The
                // byte after HALT executes twice. Clear the flag after
                // applying to prevent further repeats.
                let halt_bug = bus.is_halt_bug_active();
                if halt_bug {
                    bus.clear_halt_bug();
                } else {
                    let pc = self.registers().pc();
                    self.registers_mut().set_pc(pc.wrapping_add(1));
                }
                self.set_opcode(op);
                self.set_operand(0, 0);
                self.set_operand(1, 0);
                self.set_operand_count(0);

                let h = TABLE[op as usize];
                match h(self, bus, 0) {
                    StepResult::Exit => {}
                    StepResult::Continue => {
                        self.set_phase(Phase::ExecuteOpcode {
                            handler: h,
                            step: 1,
                        });
                    }
                }
            }
            Phase::ExecuteOpcode { handler, step } => match handler(self, bus, step) {
                StepResult::Exit => self.set_phase(Phase::FetchOpcode),
                StepResult::Continue => {
                    self.set_phase(Phase::ExecuteOpcode {
                        handler,
                        step: step + 1,
                    });
                }
            },
            Phase::InterruptDispatch { remaining } => {
                // CGB D dispatch = 5 M-cycles total.
                // FetchOpcode consumed 1, InterruptDispatch consumes 4.
                if remaining <= 1 {
                    self.set_phase(Phase::FetchOpcode);
                } else {
                    self.set_phase(Phase::InterruptDispatch {
                        remaining: remaining - 1,
                    });
                }
            }
        }
    }

    fn check_interrupts(&mut self, bus: &mut GbcMemoryBus) {
        if bus.ime_enabled()
            && let Some(kind) = bus.acknowledge_interrupt()
        {
            dispatch_interrupt(self.registers_mut(), kind, bus);
            // CGB D dispatch = 5 M-cycles total (20 T-cycles).
            // The FetchOpcode that detected the interrupt is dispatch M1.
            // Remaining 4 M-cycles as InterruptDispatch.
            self.set_phase(Phase::InterruptDispatch { remaining: 4 });
        } else {
            bus.acknowledge_interrupt();
        }
    }
}

#[cfg(test)]
mod opcode_tests;
