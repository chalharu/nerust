pub mod opcodes;
pub mod registers;

use std::fmt;

use crate::cpu::registers::CpuRegisters;
use crate::interrupt::InterruptKind;
use crate::memory::GbcMemoryBus;

/// Phases of the CPU state machine.
#[derive(Debug)]
enum Phase {
    Fetch,
    Execute(McxState),
}

/// State tracked during multi-M-cycle instruction execution.
struct McxState {
    opcode: u8,
    total_m_cycles: u8,
    current_m_cycle: u8,
    /// Operand bytes fetched from memory (max 2 for d16).
    operands: [u8; 2],
    operand_count: u8,
    byte_count: u8,
}

impl fmt::Debug for McxState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("McxState")
            .field("opcode", &format_args!("{:#04X}", self.opcode))
            .field(
                "m_cycle",
                &format_args!("{}/{}", self.current_m_cycle, self.total_m_cycles),
            )
            .field(
                "operands",
                &format_args!("{:02X?}", &self.operands[..self.operand_count as usize]),
            )
            .finish()
    }
}

pub struct Lr35902Cpu {
    pub registers: CpuRegisters,
    ime_delayed: bool,
    phase: Phase,
}

impl fmt::Debug for Lr35902Cpu {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Lr35902Cpu")
            .field("registers", &self.registers)
            .field("ime_delayed", &self.ime_delayed)
            .field("phase", &self.phase)
            .finish()
    }
}

impl Lr35902Cpu {
    pub fn new() -> Self {
        Self {
            registers: CpuRegisters::new(),
            ime_delayed: false,
            phase: Phase::Fetch,
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

        match &self.phase {
            Phase::Fetch => {
                self.check_interrupts(bus);
                self.fetch_opcode(bus);
            }
            Phase::Execute(state) => {
                let state = McxState {
                    opcode: state.opcode,
                    total_m_cycles: state.total_m_cycles,
                    current_m_cycle: state.current_m_cycle,
                    operands: state.operands,
                    operand_count: state.operand_count,
                    byte_count: state.byte_count,
                };
                self.execute_m_cycle(state, bus);
            }
        }
    }

    fn fetch_opcode(&mut self, bus: &mut GbcMemoryBus) {
        let opcode = if bus.is_dma_active() {
            bus.read_dma(self.registers.pc).unwrap_or(0xFF)
        } else {
            bus.read(self.registers.pc)
        };
        self.registers.pc = self.registers.pc.wrapping_add(1);

        let (byte_count, total_cycles) = opcode_info(opcode);
        let total_m_cycles = total_cycles as u8 / 4;

        if total_m_cycles <= 1 {
            // Single M-cycle: execute immediately
            crate::cpu::opcodes::execute(opcode, &mut self.registers, bus, [0; 2]);
            self.phase = Phase::Fetch;
        } else {
            self.phase = Phase::Execute(McxState {
                opcode,
                total_m_cycles,
                current_m_cycle: 1,
                operands: [0; 2],
                operand_count: 0,
                byte_count,
            });
        }
    }

    fn execute_m_cycle(&mut self, mut state: McxState, bus: &mut GbcMemoryBus) {
        state.current_m_cycle += 1;

        // If we still need to fetch operand bytes, do it on M-cycles 2+
        let operands_needed = state.byte_count.saturating_sub(1);

        if state.operand_count < operands_needed {
            let b = if bus.is_dma_active() {
                bus.read_dma(self.registers.pc).unwrap_or(0xFF)
            } else {
                bus.read(self.registers.pc)
            };
            state.operands[state.operand_count as usize] = b;
            state.operand_count += 1;
            self.registers.pc = self.registers.pc.wrapping_add(1);
        }

        if state.current_m_cycle == state.total_m_cycles {
            // Final M-cycle: execute the instruction
            // Reconstruct PC to point after operands (step() already advanced it)
            crate::cpu::opcodes::execute(state.opcode, &mut self.registers, bus, state.operands);
            self.phase = Phase::Fetch;
        } else {
            self.phase = Phase::Execute(state);
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

/// Return (byte_count, total_t_cycles) for an opcode.
fn opcode_info(opcode: u8) -> (u8, u32) {
    match opcode {
        0x00 | 0x40 | 0x49 | 0x52 | 0x5B | 0x64 | 0x6D | 0x7F => (1, 4), // NOP / LD r,r → 1 M-cycle
        0x76 => (1, 4),                                                  // HALT

        0x01 | 0x11 | 0x21 | 0x31 => (3, 12), // LD r16, d16
        0x08 => (3, 20),                      // LD (a16), SP

        0x02 | 0x12 => (1, 8), // LD (r16mem), A
        0x0A | 0x1A => (1, 8), // LD A, (r16mem)

        0x03 | 0x13 | 0x23 | 0x33 | 0x0B | 0x1B | 0x2B | 0x3B => (1, 8), // INC/DEC r16 + SP

        0x04 | 0x0C | 0x14 | 0x1C | 0x24 | 0x2C | 0x34 | 0x3C => (1, 4), // INC r8 / (HL)→12
        0x05 | 0x0D | 0x15 | 0x1D | 0x25 | 0x2D | 0x35 | 0x3D => (1, 4), // DEC r8 / (HL)→12

        0x06 | 0x0E | 0x16 | 0x1E | 0x26 | 0x2E | 0x36 | 0x3E => (2, 8), // LD r8, d8

        0x07 | 0x0F | 0x17 | 0x1F => (1, 4), // RLCA/RRCA/RLA/RRA
        0x09 | 0x19 | 0x29 | 0x39 => (1, 8), // ADD HL, r16

        0x10 => (2, 4), // STOP

        // JR
        0x18 => (2, 12),                     // JR e
        0x20 | 0x28 | 0x30 | 0x38 => (2, 8), // JR cc, e (not taken)

        // LD A, (HL+/-)
        0x22 | 0x32 => (1, 8), // LD (HL+/-), A
        0x2A | 0x3A => (1, 8), // LD A, (HL+/-)

        0x27 => (1, 4), // DAA
        0x2F => (1, 4), // CPL
        0x37 => (1, 4), // SCF
        0x3F => (1, 4), // CCF

        // LD r,r (1-byte, 1 M-cycle)
        0x41 | 0x42 | 0x43 | 0x44 | 0x45 | 0x46 | 0x47 | 0x48 | 0x4A | 0x4B | 0x4C | 0x4D
        | 0x4E | 0x4F | 0x50 | 0x51 | 0x53 | 0x54 | 0x55 | 0x56 | 0x57 | 0x58 | 0x59 | 0x5A
        | 0x5C | 0x5D | 0x5E | 0x5F | 0x60 | 0x61 | 0x62 | 0x63 | 0x65 | 0x66 | 0x67 | 0x68
        | 0x69 | 0x6A | 0x6B | 0x6C | 0x6E | 0x6F | 0x70 | 0x71 | 0x72 | 0x73 | 0x74 | 0x75
        | 0x77 | 0x78 | 0x79 | 0x7A | 0x7B | 0x7C | 0x7D | 0x7E => {
            let c = if matches!(
                opcode,
                0x46 | 0x4E | 0x56 | 0x5E | 0x66 | 0x6E | 0x7E | 0x70..=0x77
            ) {
                8
            } else {
                4
            };
            (1, c)
        }

        // ALU A, r8 (1-byte, HL-indirect takes 8, rest 4)
        0x80..=0xBF => {
            let c = if opcode & 0x07 == 0x06 { 8 } else { 4 };
            (1, c)
        }

        // RET cc
        0xC0 | 0xC8 | 0xD0 | 0xD8 => (1, 8), // not taken
        0xC9 => (1, 16),                     // RET
        0xD9 => (1, 16),                     // RETI

        // POP
        0xC1 | 0xD1 | 0xE1 | 0xF1 => (1, 12),

        // JP cc, a16
        0xC2 | 0xCA | 0xD2 | 0xDA => (3, 12), // not taken
        0xC3 => (3, 16),                      // JP a16

        // CALL cc, a16
        0xC4 | 0xCC | 0xD4 | 0xDC => (3, 12), // not taken
        0xCD => (3, 24),                      // CALL a16

        // PUSH
        0xC5 | 0xD5 | 0xE5 | 0xF5 => (1, 16),

        // ALU A, d8
        0xC6 | 0xCE | 0xD6 | 0xDE | 0xE6 | 0xEE | 0xF6 | 0xFE => (2, 8),

        // RST
        0xC7 | 0xCF | 0xD7 | 0xDF | 0xE7 | 0xEF | 0xF7 | 0xFF => (1, 16),

        // LDH (a8), A / LDH A, (a8)
        0xE0 | 0xF0 => (2, 12),

        // LD (C), A / LD A, (C)
        0xE2 | 0xF2 => (1, 8),

        // ADD SP, e / LD HL, SP+e
        0xE8 => (2, 16),
        0xF8 => (2, 12),

        // JP HL
        0xE9 => (1, 4),

        // LD (a16), A / LD A, (a16)
        0xEA | 0xFA => (3, 16),

        // LD SP, HL / DI / EI
        0xF9 => (1, 8),
        0xF3 => (1, 4),
        0xFB => (1, 4),

        // Invalid opcodes
        _ => (1, 4),
    }
}
