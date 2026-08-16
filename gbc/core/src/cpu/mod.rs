use std::sync::LazyLock;

use crate::cpu_core::{HandlerFn, Lr35902Cpu, Phase, StepResult};
use crate::memory::{CpuStepper, GbcMemoryBus};

static TABLE: LazyLock<[HandlerFn; 256]> = LazyLock::new(|| crate::cpu_opcodes::handler_table());

impl CpuStepper for Lr35902Cpu {
    fn tick_value(&self) -> u32 {
        0
    }

    fn sp(&self) -> u16 {
        self.registers().sp()
    }

    fn pc(&self) -> u16 {
        self.registers().pc()
    }

    /// Step one M-cycle (no device advancement — caller must call step_devices).
    fn step(&mut self, bus: &mut GbcMemoryBus) {
        if bus.is_halted_or_stopped() {
            self.check_interrupts(bus);
            if bus.is_halted_or_stopped() {
                return;
            }
            // An interrupt was just dispatched (HALT woke up). The detection
            // M-cycle is consumed here; the InterruptDispatch phase runs on
            // the next step, matching the real hardware 5 M-cycle dispatch.
            if matches!(self.phase(), Phase::InterruptDispatch { .. }) {
                return;
            }
            // IME=0 wake: no dispatch happens, so the instruction after HALT
            // starts on this M-cycle (no extra delay), exactly as if a series
            // of NOPs had been waiting for the interrupt.
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
                self.arm_delayed_ime();
                let fetch_pc = self.registers().pc();
                bus.set_current_pc(fetch_pc);
                let op = bus.read(fetch_pc);
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
                    StepResult::Exit => self.finish_instruction(bus),
                    StepResult::Continue => {
                        self.set_phase(Phase::ExecuteOpcode {
                            handler: h,
                            step: 1,
                        });
                    }
                }
            }
            Phase::ExecuteOpcode { handler, step } => match handler(self, bus, step) {
                StepResult::Exit => {
                    self.set_phase(Phase::FetchOpcode);
                    self.finish_instruction(bus);
                }
                StepResult::Continue => {
                    self.set_phase(Phase::ExecuteOpcode {
                        handler,
                        step: step + 1,
                    });
                }
            },
            Phase::InterruptDispatch {
                step,
                pending_ie,
                pending_if,
            } => match step {
                1 => self.set_phase(Phase::InterruptDispatch {
                    step: 2,
                    pending_ie,
                    pending_if,
                }),
                2 => {
                    let sp = self.registers().sp().wrapping_sub(1);
                    self.registers_mut().set_sp(sp);
                    bus.write(sp, (self.registers().pc() >> 8) as u8);
                    // The high-byte push may have written to the IE register
                    // ($FFFF); snapshot IE now for the dispatch decision.
                    let pending_ie = bus.read_ie();
                    self.set_phase(Phase::InterruptDispatch {
                        step: 3,
                        pending_ie,
                        pending_if,
                    });
                }
                3 => {
                    let sp = self.registers().sp().wrapping_sub(1);
                    self.registers_mut().set_sp(sp);
                    // If the low-byte push targets the IF register ($FF0F),
                    // the dispatch decision uses the pre-write IF value.
                    let pending_if = bus.read_if_raw();
                    bus.write(sp, self.registers().pc() as u8);
                    self.set_phase(Phase::InterruptDispatch {
                        step: 4,
                        pending_ie,
                        pending_if,
                    });
                }
                4 => {
                    // Re-evaluate IE & IF after the pushes (the pushes can
                    // modify IE/IF, cancelling or changing the dispatch).
                    let queue = pending_ie & pending_if & 0x1F;
                    if queue != 0 {
                        let n = queue.trailing_zeros() as u8;
                        bus.clear_if_bit(n);
                        self.registers_mut().set_pc((n as u16) * 8 + 0x40);
                    } else {
                        // Dispatch cancelled: PC is set to 0, IF untouched.
                        self.registers_mut().set_pc(0);
                    }
                    self.set_phase(Phase::FetchOpcode);
                }
                _ => unreachable!("invalid interrupt dispatch step"),
            },
        }
    }
}

impl Lr35902Cpu {
    fn finish_instruction(&mut self, bus: &mut GbcMemoryBus) {
        if self.take_armed_ime() {
            bus.set_ime(true);
        }
    }

    fn check_interrupts(&mut self, bus: &mut GbcMemoryBus) {
        if bus.ime_enabled()
            && let Some(_kind) = bus.acknowledge_interrupt()
        {
            self.set_phase(Phase::InterruptDispatch {
                step: 1,
                pending_ie: 0,
                pending_if: 0,
            });
        } else {
            bus.acknowledge_interrupt();
        }
    }
}

#[cfg(test)]
mod opcode_tests;
