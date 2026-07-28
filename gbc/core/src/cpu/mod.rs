use std::sync::LazyLock;

use crate::cpu_core::{HandlerFn, Lr35902Cpu, Phase, StepResult};
use crate::interrupt::dispatch_interrupt;
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
            return;
        }

        match self.phase() {
            Phase::FetchOpcode => {
                self.check_interrupts(bus);
                let op = bus.read(self.registers().pc());
                let pc = self.registers().pc();
                self.registers_mut().set_pc(pc.wrapping_add(1));
                self.set_opcode(op);
                self.set_operand(0, 0);
                self.set_operand(1, 0);
                self.set_operand_count(0);

                let h = TABLE[op as usize];
                // step=0 signals "fetch decode" to the handler
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
        }
    }

    fn check_interrupts(&mut self, bus: &mut GbcMemoryBus) {
        if bus.ime_enabled()
            && let Some(kind) = bus.acknowledge_interrupt()
        {
            dispatch_interrupt(self.registers_mut(), kind, bus);
        }
    }
}

#[cfg(test)]
mod opcode_tests;
