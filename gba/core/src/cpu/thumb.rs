use crate::cpu::registers::CpuRegisters;
use crate::memory::GbaMemoryBus;

/// Thumbデコーダ。Format 1-19 で振り分け。
pub fn decode_thumb(_regs: &mut CpuRegisters, _bus: &mut GbaMemoryBus, instr: u16) -> u32 {
    let op = instr >> 11;
    match op {
        0b00000..=0b00010 => 1,                          // Move shifted register
        0b00011 => 1,                                    // Add/subtract
        0b00100..=0b00111 => 1,                          // Move/compare/add/sub immediate
        0b01000 if (instr >> 6) & 0x3F == 0b000000 => 1, // ALU operations
        0b01000..=0b01001 => 1, // Hi register operations / BX, PC-relative load
        0b01010..=0b01011 => 1, // Load/store with register offset, sign-extended
        0b01100..=0b01111 => 1, // Load/store with immediate offset
        0b10000 => 1,           // Load/store halfword
        0b10010..=0b10011 => 1, // SP-relative, Load address
        0b10110 => 1,           // Add offset to SP, Push/pop
        0b11000 => 1,           // Multiple load/store
        0b11010..=0b11011 => 1, // Conditional branch / SWI
        0b11100 => 1,           // Unconditional branch
        0b11110..=0b11111 => 1, // Long branch with link
        _ => 1,
    }
}
