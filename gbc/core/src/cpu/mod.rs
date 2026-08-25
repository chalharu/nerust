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
        if bus.is_halted_or_stopped() && self.try_halt_wake(bus) {
            return;
        }

        match self.phase() {
            Phase::FetchOpcode => self.step_fetch_opcode(bus),
            Phase::ExecuteOpcode { handler, step } => self.step_execute_opcode(handler, step, bus),
            Phase::InterruptDispatch {
                step,
                pending_ie,
                pending_if,
            } => self.step_interrupt_dispatch(step, pending_ie, pending_if, bus),
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

    fn try_halt_wake(&mut self, bus: &mut GbcMemoryBus) -> bool {
        self.check_interrupts(bus);
        if bus.is_halted_or_stopped() {
            return true;
        }
        // An interrupt was just dispatched (HALT woke up). The detection
        // M-cycle is consumed here; the InterruptDispatch phase runs on
        // the next step, matching the real hardware 5 M-cycle dispatch.
        matches!(self.phase(), Phase::InterruptDispatch { .. })
    }

    fn step_fetch_opcode(&mut self, bus: &mut GbcMemoryBus) {
        self.check_interrupts(bus);
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
        if bus.is_halt_bug_active() {
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

    fn step_execute_opcode(
        &mut self,
        handler: HandlerFn,
        step: u8,
        bus: &mut GbcMemoryBus,
    ) {
        match handler(self, bus, step) {
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
        }
    }

    fn step_interrupt_dispatch(
        &mut self,
        step: u8,
        pending_ie: u8,
        pending_if: u8,
        bus: &mut GbcMemoryBus,
    ) {
        let next = |cpu: &mut Self, step, pending_ie, pending_if| {
            cpu.set_phase(Phase::InterruptDispatch {
                step,
                pending_ie,
                pending_if,
            });
        };
        match step {
            1 => next(self, 2, pending_ie, pending_if),
            2 => {
                let sp = self.registers().sp().wrapping_sub(1);
                self.registers_mut().set_sp(sp);
                bus.write(sp, (self.registers().pc() >> 8) as u8);
                let pending_ie = bus.read_ie();
                next(self, 3, pending_ie, pending_if);
            }
            3 => {
                let sp = self.registers().sp().wrapping_sub(1);
                self.registers_mut().set_sp(sp);
                let pending_if = bus.read_if_raw();
                bus.write(sp, self.registers().pc() as u8);
                next(self, 4, pending_ie, pending_if);
            }
            4 => {
                let queue = pending_ie & pending_if & 0x1F;
                if queue != 0 {
                    let n = queue.trailing_zeros() as u8;
                    bus.clear_if_bit(n);
                    self.registers_mut().set_pc((n as u16) * 8 + 0x40);
                } else {
                    self.registers_mut().set_pc(0);
                }
                self.set_phase(Phase::FetchOpcode);
            }
            _ => unreachable!("invalid interrupt dispatch step"),
        }
    }
}

#[cfg(test)]
mod opcode_tests;
