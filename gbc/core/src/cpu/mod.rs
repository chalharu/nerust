pub mod opcodes;
pub mod registers;

use crate::cpu::opcodes::execute;
use crate::cpu::registers::CpuRegisters;
use crate::memory::GbcMemoryBus;

/// LR35902 CPU (Game Boy / GBC).
///
/// Executes one instruction per `step()` call, returning consumed T-cycles.
/// Interrupt dispatch, HALT/STOP detection, and DMA-mode switching are
/// handled here.
pub struct Lr35902Cpu {
    pub registers: CpuRegisters,
    ime: bool,
    ime_delayed: bool,
    halted: bool,
}

impl Lr35902Cpu {
    pub fn new() -> Self {
        Self {
            registers: CpuRegisters::new(),
            ime: false,
            ime_delayed: false,
            halted: false,
        }
    }

    /// Execute one instruction. Returns number of T-cycles consumed.
    pub fn step(&mut self, bus: &mut GbcMemoryBus) -> u32 {
        // Handle IME delay (EI sets IME after next instruction)
        if self.ime_delayed {
            self.ime = true;
            self.ime_delayed = false;
        }

        if bus.is_halted_or_stopped() {
            return 4;
        }

        // Check for interrupt dispatch
        if self.ime
            && let Some(kind) = bus.acknowledge_interrupt()
        {
            self.dispatch_interrupt(kind, bus);
            return 20;
        }

        self.execute_one(bus)
    }

    /// Execute one opcode. Handles DMA mode restrictions.
    fn execute_one(&mut self, bus: &mut GbcMemoryBus) -> u32 {
        if bus.is_dma_active() {
            let opcode = bus.read_dma(self.registers.pc).unwrap_or(0xFF);
            self.registers.pc = self.registers.pc.wrapping_add(1);
            return execute(opcode, &mut self.registers, bus);
        }

        let opcode = bus.read(self.registers.pc);
        self.registers.pc = self.registers.pc.wrapping_add(1);
        execute(opcode, &mut self.registers, bus)
    }

    fn dispatch_interrupt(
        &mut self,
        kind: crate::interrupt::InterruptKind,
        bus: &mut GbcMemoryBus,
    ) {
        // Push PC to stack
        let pc = self.registers.pc;
        self.registers.sp = self.registers.sp.wrapping_sub(1);
        bus.write(self.registers.sp, (pc >> 8) as u8);
        self.registers.sp = self.registers.sp.wrapping_sub(1);
        bus.write(self.registers.sp, pc as u8);

        // Jump to interrupt vector
        self.registers.pc = kind.vector();
    }
}

impl Default for Lr35902Cpu {
    fn default() -> Self {
        Self::new()
    }
}
