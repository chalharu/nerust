pub mod opcodes;
pub mod registers;

use crate::cpu::opcodes::execute;
use crate::cpu::registers::CpuRegisters;
use crate::interrupt::InterruptKind;
use crate::memory::GbcMemoryBus;

/// LR35902 CPU (Game Boy / GBC).
///
/// `step()` executes exactly **one M-cycle** (= 4 T-cycles in normal speed).
/// Multi-M-cycle instructions are split across multiple `step()` calls so
/// that interrupts and devices are updated at M-cycle boundaries.
pub struct Lr35902Cpu {
    pub registers: CpuRegisters,
    ime_delayed: bool,
    /// Remaining M-cycles for the current instruction.
    /// When 0, the next step() fetches a new opcode.
    /// When >0, the next step() counts down and executes when it reaches 0.
    remaining_m_cycles: u8,
    /// Pending opcode (fetched but not yet executed).
    pending_opcode: u8,
}

impl Lr35902Cpu {
    pub fn new() -> Self {
        Self {
            registers: CpuRegisters::new(),
            ime_delayed: false,
            remaining_m_cycles: 0,
            pending_opcode: 0,
        }
    }

    /// Execute one M-cycle (= 4 T-cycles).
    ///
    /// On the first M-cycle of a new instruction: checks interrupts,
    /// handles IME delay, and fetches the opcode byte. The instruction
    /// executes atomically on its final M-cycle.
    pub fn step(&mut self, bus: &mut GbcMemoryBus) {
        // IME delay: EI sets IME after the NEXT instruction.
        if self.remaining_m_cycles == 0 && self.ime_delayed {
            bus.set_ime(true);
            self.ime_delayed = false;
        }

        if bus.is_halted_or_stopped() {
            return;
        }

        // Check interrupts between M-cycles (if at instruction boundary)
        if self.remaining_m_cycles == 0 {
            self.check_interrupts(bus);
        }

        if self.remaining_m_cycles == 0 {
            // ── New instruction ──
            let opcode = if bus.is_dma_active() {
                bus.read_dma(self.registers.pc).unwrap_or(0xFF)
            } else {
                bus.read(self.registers.pc)
            };
            self.registers.pc = self.registers.pc.wrapping_add(1);

            let cycles = execute(opcode, &mut self.registers, bus);
            let m_cycles = cycles / 4;

            if m_cycles <= 1 {
                return;
            }

            self.remaining_m_cycles = (m_cycles - 1) as u8;
        } else {
            // ── Continue multi-M-cycle instruction ──
            self.remaining_m_cycles -= 1;
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
        let pc = self.registers.pc;
        self.registers.sp = self.registers.sp.wrapping_sub(1);
        bus.write(self.registers.sp, (pc >> 8) as u8);
        self.registers.sp = self.registers.sp.wrapping_sub(1);
        bus.write(self.registers.sp, pc as u8);
        self.registers.pc = kind.vector();
    }
}

impl Default for Lr35902Cpu {
    fn default() -> Self {
        Self::new()
    }
}
