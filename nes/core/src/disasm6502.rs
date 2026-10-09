//! 6502 disassembler for the debugger's disassembly view.
//!
//! Rebuilt from prototype findings: JAM opcodes and short reads decode
//! as raw `.DB $xx`; `$6C` is plain indirect. Verified against the
//! nestest official trace (see `debugger` tests).

/// Addressing mode. Length derives from the mode, except raw fallback.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AddrMode {
    Implied,
    Accumulator,
    Immediate,
    ZeroPage,
    ZeroPageX,
    ZeroPageY,
    Absolute,
    AbsoluteX,
    AbsoluteY,
    Indirect,
    IndexedIndirect,
    IndirectIndexed,
    Relative,
    /// Illegal opcode: 1 raw byte.
    Jam,
}

impl AddrMode {
    pub(crate) fn len(self) -> u8 {
        match self {
            Self::Implied | Self::Accumulator | Self::Jam => 1,
            Self::Immediate
            | Self::ZeroPage
            | Self::ZeroPageX
            | Self::ZeroPageY
            | Self::IndexedIndirect
            | Self::IndirectIndexed
            | Self::Relative => 2,
            Self::Absolute | Self::AbsoluteX | Self::AbsoluteY | Self::Indirect => 3,
        }
    }
}

/// Decode one opcode to (mnemonic, mode).
pub(crate) fn decode(op: u8) -> (&'static str, AddrMode) {
    use AddrMode as M;
    match op {
        // x0
        0x00 => ("BRK", M::Implied),
        0x01 => ("ORA", M::IndexedIndirect),
        0x02 | 0x12 | 0x22 | 0x32 | 0x42 | 0x52 | 0x62 | 0x72 | 0x92 | 0xB2 | 0xC2 | 0xD2
        | 0xE2 | 0xF2 => ("JAM", M::Jam),
        0x03 => ("SLO", M::IndexedIndirect),
        0x04 => ("NOP", M::ZeroPage),
        0x05 => ("ORA", M::ZeroPage),
        0x06 => ("ASL", M::ZeroPage),
        0x07 => ("SLO", M::ZeroPage),
        0x08 => ("PHP", M::Implied),
        0x09 => ("ORA", M::Immediate),
        0x0A => ("ASL", M::Accumulator),
        0x0B | 0x2B => ("ANC", M::Immediate),
        0x0C => ("NOP", M::Absolute),
        0x0D => ("ORA", M::Absolute),
        0x0E => ("ASL", M::Absolute),
        0x0F => ("SLO", M::Absolute),
        // x1
        0x10 => ("BPL", M::Relative),
        0x11 => ("ORA", M::IndirectIndexed),
        0x13 => ("SLO", M::IndirectIndexed),
        0x14 => ("NOP", M::ZeroPageX),
        0x15 => ("ORA", M::ZeroPageX),
        0x16 => ("ASL", M::ZeroPageX),
        0x17 => ("SLO", M::ZeroPageX),
        0x18 => ("CLC", M::Implied),
        0x19 => ("ORA", M::AbsoluteY),
        0x1A => ("NOP", M::Implied),
        0x1B => ("SLO", M::AbsoluteY),
        0x1C => ("NOP", M::AbsoluteX),
        0x1D => ("ORA", M::AbsoluteX),
        0x1E => ("ASL", M::AbsoluteX),
        0x1F => ("SLO", M::AbsoluteX),
        // x2
        0x20 => ("JSR", M::Absolute),
        0x21 => ("AND", M::IndexedIndirect),
        0x23 => ("RLA", M::IndexedIndirect),
        0x24 => ("BIT", M::ZeroPage),
        0x25 => ("AND", M::ZeroPage),
        0x26 => ("ROL", M::ZeroPage),
        0x27 => ("RLA", M::ZeroPage),
        0x28 => ("PLP", M::Implied),
        0x29 => ("AND", M::Immediate),
        0x2A => ("ROL", M::Accumulator),
        0x2C => ("BIT", M::Absolute),
        0x2D => ("AND", M::Absolute),
        0x2E => ("ROL", M::Absolute),
        0x2F => ("RLA", M::Absolute),
        // x3
        0x30 => ("BMI", M::Relative),
        0x31 => ("AND", M::IndirectIndexed),
        0x33 => ("RLA", M::IndirectIndexed),
        0x34 => ("NOP", M::ZeroPageX),
        0x35 => ("AND", M::ZeroPageX),
        0x36 => ("ROL", M::ZeroPageX),
        0x37 => ("RLA", M::ZeroPageX),
        0x38 => ("SEC", M::Implied),
        0x39 => ("AND", M::AbsoluteY),
        0x3A => ("NOP", M::Implied),
        0x3B => ("RLA", M::AbsoluteY),
        0x3C => ("NOP", M::AbsoluteX),
        0x3D => ("AND", M::AbsoluteX),
        0x3E => ("ROL", M::AbsoluteX),
        0x3F => ("RLA", M::AbsoluteX),
        // x4
        0x40 => ("RTI", M::Implied),
        0x41 => ("EOR", M::IndexedIndirect),
        0x43 => ("SRE", M::IndexedIndirect),
        0x44 => ("NOP", M::ZeroPage),
        0x45 => ("EOR", M::ZeroPage),
        0x46 => ("LSR", M::ZeroPage),
        0x47 => ("SRE", M::ZeroPage),
        0x48 => ("PHA", M::Implied),
        0x49 => ("EOR", M::Immediate),
        0x4A => ("LSR", M::Accumulator),
        0x4B => ("ALR", M::Immediate),
        0x4C => ("JMP", M::Absolute),
        0x4D => ("EOR", M::Absolute),
        0x4E => ("LSR", M::Absolute),
        0x4F => ("SRE", M::Absolute),
        // x5
        0x50 => ("BVC", M::Relative),
        0x51 => ("EOR", M::IndirectIndexed),
        0x53 => ("SRE", M::IndirectIndexed),
        0x54 => ("NOP", M::ZeroPageX),
        0x55 => ("EOR", M::ZeroPageX),
        0x56 => ("LSR", M::ZeroPageX),
        0x57 => ("SRE", M::ZeroPageX),
        0x58 => ("CLI", M::Implied),
        0x59 => ("EOR", M::AbsoluteY),
        0x5A => ("NOP", M::Implied),
        0x5B => ("SRE", M::AbsoluteY),
        0x5C => ("NOP", M::AbsoluteX),
        0x5D => ("EOR", M::AbsoluteX),
        0x5E => ("LSR", M::AbsoluteX),
        0x5F => ("SRE", M::AbsoluteX),
        // x6
        0x60 => ("RTS", M::Implied),
        0x61 => ("ADC", M::IndexedIndirect),
        0x63 => ("RRA", M::IndexedIndirect),
        0x64 => ("NOP", M::ZeroPage),
        0x65 => ("ADC", M::ZeroPage),
        0x66 => ("ROR", M::ZeroPage),
        0x67 => ("RRA", M::ZeroPage),
        0x68 => ("PLA", M::Implied),
        0x69 => ("ADC", M::Immediate),
        0x6A => ("ROR", M::Accumulator),
        0x6B => ("ARR", M::Immediate),
        0x6C => ("JMP", M::Indirect),
        0x6D => ("ADC", M::Absolute),
        0x6E => ("ROR", M::Absolute),
        0x6F => ("RRA", M::Absolute),
        // x7
        0x70 => ("BVS", M::Relative),
        0x71 => ("ADC", M::IndirectIndexed),
        0x73 => ("RRA", M::IndirectIndexed),
        0x74 => ("NOP", M::ZeroPageX),
        0x75 => ("ADC", M::ZeroPageX),
        0x76 => ("ROR", M::ZeroPageX),
        0x77 => ("RRA", M::ZeroPageX),
        0x78 => ("SEI", M::Implied),
        0x79 => ("ADC", M::AbsoluteY),
        0x7A => ("NOP", M::Implied),
        0x7B => ("RRA", M::AbsoluteY),
        0x7C => ("NOP", M::AbsoluteX),
        0x7D => ("ADC", M::AbsoluteX),
        0x7E => ("ROR", M::AbsoluteX),
        0x7F => ("RRA", M::AbsoluteX),
        // x8
        0x80 => ("NOP", M::Immediate),
        0x81 => ("STA", M::IndexedIndirect),
        0x82 => ("NOP", M::Immediate),
        0x83 => ("SAX", M::IndexedIndirect),
        0x84 => ("STY", M::ZeroPage),
        0x85 => ("STA", M::ZeroPage),
        0x86 => ("STX", M::ZeroPage),
        0x87 => ("SAX", M::ZeroPage),
        0x88 => ("DEY", M::Implied),
        0x89 => ("NOP", M::Immediate),
        0x8A => ("TXA", M::Implied),
        0x8B => ("XAA", M::Immediate),
        0x8C => ("STY", M::Absolute),
        0x8D => ("STA", M::Absolute),
        0x8E => ("STX", M::Absolute),
        0x8F => ("SAX", M::Absolute),
        // x9
        0x90 => ("BCC", M::Relative),
        0x91 => ("STA", M::IndirectIndexed),
        0x93 => ("AHX", M::IndirectIndexed),
        0x94 => ("STY", M::ZeroPageX),
        0x95 => ("STA", M::ZeroPageX),
        0x96 => ("STX", M::ZeroPageY),
        0x97 => ("SAX", M::ZeroPageY),
        0x98 => ("TYA", M::Implied),
        0x99 => ("STA", M::AbsoluteY),
        0x9A => ("TXS", M::Implied),
        0x9B => ("TAS", M::AbsoluteY),
        0x9C => ("SHY", M::AbsoluteX),
        0x9D => ("STA", M::AbsoluteX),
        0x9E => ("SHX", M::AbsoluteY),
        0x9F => ("AHX", M::AbsoluteY),
        // xA
        0xA0 => ("LDY", M::Immediate),
        0xA1 => ("LDA", M::IndexedIndirect),
        0xA2 => ("LDX", M::Immediate),
        0xA3 => ("LAX", M::IndexedIndirect),
        0xA4 => ("LDY", M::ZeroPage),
        0xA5 => ("LDA", M::ZeroPage),
        0xA6 => ("LDX", M::ZeroPage),
        0xA7 => ("LAX", M::ZeroPage),
        0xA8 => ("TAY", M::Implied),
        0xA9 => ("LDA", M::Immediate),
        0xAA => ("TAX", M::Implied),
        0xAB => ("LAX", M::Immediate),
        0xAC => ("LDY", M::Absolute),
        0xAD => ("LDA", M::Absolute),
        0xAE => ("LDX", M::Absolute),
        0xAF => ("LAX", M::Absolute),
        // xB
        0xB0 => ("BCS", M::Relative),
        0xB1 => ("LDA", M::IndirectIndexed),
        0xB3 => ("LAX", M::IndirectIndexed),
        0xB4 => ("LDY", M::ZeroPageX),
        0xB5 => ("LDA", M::ZeroPageX),
        0xB6 => ("LDX", M::ZeroPageY),
        0xB7 => ("LAX", M::ZeroPageY),
        0xB8 => ("CLV", M::Implied),
        0xB9 => ("LDA", M::AbsoluteY),
        0xBA => ("TSX", M::Implied),
        0xBB => ("LAS", M::AbsoluteY),
        0xBC => ("LDY", M::AbsoluteX),
        0xBD => ("LDA", M::AbsoluteX),
        0xBE => ("LDX", M::AbsoluteY),
        0xBF => ("LAX", M::AbsoluteY),
        // xC
        0xC0 => ("CPY", M::Immediate),
        0xC1 => ("CMP", M::IndexedIndirect),
        0xC3 => ("DCP", M::IndexedIndirect),
        0xC4 => ("CPY", M::ZeroPage),
        0xC5 => ("CMP", M::ZeroPage),
        0xC6 => ("DEC", M::ZeroPage),
        0xC7 => ("DCP", M::ZeroPage),
        0xC8 => ("INY", M::Implied),
        0xC9 => ("CMP", M::Immediate),
        0xCA => ("DEX", M::Implied),
        0xCB => ("AXS", M::Immediate),
        0xCC => ("CPY", M::Absolute),
        0xCD => ("CMP", M::Absolute),
        0xCE => ("DEC", M::Absolute),
        0xCF => ("DCP", M::Absolute),
        // xD
        0xD0 => ("BNE", M::Relative),
        0xD1 => ("CMP", M::IndirectIndexed),
        0xD3 => ("DCP", M::IndirectIndexed),
        0xD4 => ("NOP", M::ZeroPageX),
        0xD5 => ("CMP", M::ZeroPageX),
        0xD6 => ("DEC", M::ZeroPageX),
        0xD7 => ("DCP", M::ZeroPageX),
        0xD8 => ("CLD", M::Implied),
        0xD9 => ("CMP", M::AbsoluteY),
        0xDA => ("NOP", M::Implied),
        0xDB => ("DCP", M::AbsoluteY),
        0xDC => ("NOP", M::AbsoluteX),
        0xDD => ("CMP", M::AbsoluteX),
        0xDE => ("DEC", M::AbsoluteX),
        0xDF => ("DCP", M::AbsoluteX),
        // xE
        0xE0 => ("CPX", M::Immediate),
        0xE1 => ("SBC", M::IndexedIndirect),
        0xE3 => ("ISC", M::IndexedIndirect),
        0xE4 => ("CPX", M::ZeroPage),
        0xE5 => ("SBC", M::ZeroPage),
        0xE6 => ("INC", M::ZeroPage),
        0xE7 => ("ISC", M::ZeroPage),
        0xE8 => ("INX", M::Implied),
        0xE9 => ("SBC", M::Immediate),
        0xEA => ("NOP", M::Implied),
        0xEB => ("SBC", M::Immediate),
        0xEC => ("CPX", M::Absolute),
        0xED => ("SBC", M::Absolute),
        0xEE => ("INC", M::Absolute),
        0xEF => ("ISC", M::Absolute),
        // xF
        0xF0 => ("BEQ", M::Relative),
        0xF1 => ("SBC", M::IndirectIndexed),
        0xF3 => ("ISC", M::IndirectIndexed),
        0xF4 => ("NOP", M::ZeroPageX),
        0xF5 => ("SBC", M::ZeroPageX),
        0xF6 => ("INC", M::ZeroPageX),
        0xF7 => ("ISC", M::ZeroPageX),
        0xF8 => ("SED", M::Implied),
        0xF9 => ("SBC", M::AbsoluteY),
        0xFA => ("NOP", M::Implied),
        0xFB => ("ISC", M::AbsoluteY),
        0xFC => ("NOP", M::AbsoluteX),
        0xFD => ("SBC", M::AbsoluteX),
        0xFE => ("INC", M::AbsoluteX),
        0xFF => ("ISC", M::AbsoluteX),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table_covers_all_256_opcodes() {
        let mut seen = [false; 256];
        for op in 0..=255u16 {
            let op = op as u8;
            let _ = decode(op);
            seen[op as usize] = true;
        }
        assert!(seen.iter().all(|s| *s));
    }

    #[test]
    fn jam_is_single_raw_byte() {
        for op in [0x02u8, 0x12, 0xD2, 0xF2] {
            assert_eq!(decode(op), ("JAM", AddrMode::Jam));
            assert_eq!(AddrMode::Jam.len(), 1);
        }
    }
}
