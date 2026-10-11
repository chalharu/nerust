//! LR35902 (SM83) disassembler for the debugger's disassembly view.
//!
//! Table-driven: every primary opcode maps to (mnemonic, operand
//! shape); `CB`-prefixed opcodes decode from bitfields. Short reads at
//! the range end decode as single raw bytes (see `raw_db` handling in
//! `debugger.rs`), so a truncated tail never panics and never
//! fabricates instructions. Illegal opcodes (`0xD3` etc.) decode as
//! `.DB $xx`, mirroring the 6502 `JAM` fallback.

/// Operand shape. Length derives from the shape.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Operand {
    None,
    Imm8,
    Imm16,
    Rel8,
}

impl Operand {
    pub(crate) fn len(self) -> u8 {
        match self {
            Self::None => 1,
            Self::Imm8 | Self::Rel8 => 2,
            Self::Imm16 => 3,
        }
    }
}

/// One decoded row: text, byte length, and follow target (if a plain
/// code address).
pub(crate) struct Decoded {
    pub text: String,
    pub len: u8,
    pub target: Option<u32>,
}

/// Instruction length without formatting (for short-tail checks).
pub(crate) fn instr_len(op: u8) -> u8 {
    shape(op).1.len()
}

/// Full decode; `b1`/`b2` are the following bytes (callers check
/// availability first and emit raw rows instead when short).
pub(crate) fn decode(addr: u32, op: u8, b1: u8, b2: u8) -> Decoded {
    if op == 0xCB {
        let (text, len) = (decode_cb(b1), 2);
        return Decoded {
            text,
            len,
            target: None,
        };
    }
    let (name, operand) = shape(op);
    let len = operand.len();
    let nn = u16::from_le_bytes([b1, b2]);
    let rel = |base: u32| base.wrapping_add(2).wrapping_add(b1 as i8 as i32 as u32) & 0xFFFF;
    // Special forms whose operand is not a plain suffix.
    let special = match op {
        0x08 => Some((format!("LD (${nn:04X}),SP"), None)),
        0x10 => Some(("STOP".to_string(), None)),
        0x18 | 0x20 | 0x28 | 0x30 | 0x38 => {
            let dest = rel(addr);
            Some((format!("{name} ${dest:04X}"), Some(dest)))
        }
        0xC2 | 0xC3 | 0xCA | 0xD2 | 0xDA | 0xC4 | 0xCC | 0xCD | 0xD4 | 0xDC => {
            Some((format!("{name} ${nn:04X}"), Some(u32::from(nn))))
        }
        0xC7 | 0xCF | 0xD7 | 0xDF | 0xE7 | 0xEF | 0xF7 | 0xFF => {
            let vec = match op {
                0xC7 => 0x00,
                0xCF => 0x08,
                0xD7 => 0x10,
                0xDF => 0x18,
                0xE7 => 0x20,
                0xEF => 0x28,
                0xF7 => 0x30,
                _ => 0x38,
            };
            Some((format!("RST ${vec:02X}"), Some(vec)))
        }
        0xE0 => Some((format!("LDH (${:04X}),A", 0xFF00 | u16::from(b1)), None)),
        0xE8 => Some((format!("ADD SP,${b1:02X}"), None)),
        0xF0 => Some((format!("LDH A,(${:04X})", 0xFF00 | u16::from(b1)), None)),
        0xF8 => Some((format!("LD HL,SP+${b1:02X}"), None)),
        // Illegal: raw byte, never fabricated.
        0xD3 | 0xDB | 0xDD | 0xE3 | 0xE4 | 0xEB | 0xEC | 0xED | 0xF4 | 0xFC | 0xFD => {
            Some((format!(".DB ${op:02X}"), None))
        }
        _ => None,
    };
    if let Some((text, target)) = special {
        return Decoded { text, len, target };
    }
    let text = match operand {
        Operand::None => name.to_string(),
        Operand::Imm8 => format!("{name} ${b1:02X}"),
        Operand::Imm16 => format!("{name} ${nn:04X}"),
        Operand::Rel8 => {
            let dest = rel(addr);
            format!("{name} ${dest:04X}")
        }
    };
    Decoded {
        text,
        len,
        target: None,
    }
}

/// Decode one primary opcode to (mnemonic, operand shape).
fn shape(op: u8) -> (&'static str, Operand) {
    use Operand as O;
    match op {
        // x0
        0x00 => ("NOP", O::None),
        0x01 => ("LD BC", O::Imm16),
        0x02 => ("LD (BC),A", O::None),
        0x03 => ("INC BC", O::None),
        0x04 => ("INC B", O::None),
        0x05 => ("DEC B", O::None),
        0x06 => ("LD B", O::Imm8),
        0x07 => ("RLCA", O::None),
        0x08 => ("LD (nn),SP", O::Imm16),
        0x09 => ("ADD HL,BC", O::None),
        0x0A => ("LD A,(BC)", O::None),
        0x0B => ("DEC BC", O::None),
        0x0C => ("INC C", O::None),
        0x0D => ("DEC C", O::None),
        0x0E => ("LD C", O::Imm8),
        0x0F => ("RRCA", O::None),
        // x1
        0x10 => ("STOP", O::Imm8),
        0x11 => ("LD DE", O::Imm16),
        0x12 => ("LD (DE),A", O::None),
        0x13 => ("INC DE", O::None),
        0x14 => ("INC D", O::None),
        0x15 => ("DEC D", O::None),
        0x16 => ("LD D", O::Imm8),
        0x17 => ("RLA", O::None),
        0x18 => ("JR", O::Rel8),
        0x19 => ("ADD HL,DE", O::None),
        0x1A => ("LD A,(DE)", O::None),
        0x1B => ("DEC DE", O::None),
        0x1C => ("INC E", O::None),
        0x1D => ("DEC E", O::None),
        0x1E => ("LD E", O::Imm8),
        0x1F => ("RRA", O::None),
        // x2
        0x20 => ("JR NZ", O::Rel8),
        0x21 => ("LD HL", O::Imm16),
        0x22 => ("LD (HL+),A", O::None),
        0x23 => ("INC HL", O::None),
        0x24 => ("INC H", O::None),
        0x25 => ("DEC H", O::None),
        0x26 => ("LD H", O::Imm8),
        0x27 => ("DAA", O::None),
        0x28 => ("JR Z", O::Rel8),
        0x29 => ("ADD HL,HL", O::None),
        0x2A => ("LD A,(HL+)", O::None),
        0x2B => ("DEC HL", O::None),
        0x2C => ("INC L", O::None),
        0x2D => ("DEC L", O::None),
        0x2E => ("LD L", O::Imm8),
        0x2F => ("CPL", O::None),
        // x3
        0x30 => ("JR NC", O::Rel8),
        0x31 => ("LD SP", O::Imm16),
        0x32 => ("LD (HL-),A", O::None),
        0x33 => ("INC SP", O::None),
        0x34 => ("INC (HL)", O::None),
        0x35 => ("DEC (HL)", O::None),
        0x36 => ("LD (HL)", O::Imm8),
        0x37 => ("SCF", O::None),
        0x38 => ("JR C", O::Rel8),
        0x39 => ("ADD HL,SP", O::None),
        0x3A => ("LD A,(HL-)", O::None),
        0x3B => ("DEC SP", O::None),
        0x3C => ("INC A", O::None),
        0x3D => ("DEC A", O::None),
        0x3E => ("LD A", O::Imm8),
        0x3F => ("CCF", O::None),
        // x4-x7: LD r,r'
        0x40 => ("LD B,B", O::None),
        0x41 => ("LD B,C", O::None),
        0x42 => ("LD B,D", O::None),
        0x43 => ("LD B,E", O::None),
        0x44 => ("LD B,H", O::None),
        0x45 => ("LD B,L", O::None),
        0x46 => ("LD B,(HL)", O::None),
        0x47 => ("LD B,A", O::None),
        0x48 => ("LD C,B", O::None),
        0x49 => ("LD C,C", O::None),
        0x4A => ("LD C,D", O::None),
        0x4B => ("LD C,E", O::None),
        0x4C => ("LD C,H", O::None),
        0x4D => ("LD C,L", O::None),
        0x4E => ("LD C,(HL)", O::None),
        0x4F => ("LD C,A", O::None),
        0x50 => ("LD D,B", O::None),
        0x51 => ("LD D,C", O::None),
        0x52 => ("LD D,D", O::None),
        0x53 => ("LD D,E", O::None),
        0x54 => ("LD D,H", O::None),
        0x55 => ("LD D,L", O::None),
        0x56 => ("LD D,(HL)", O::None),
        0x57 => ("LD D,A", O::None),
        0x58 => ("LD E,B", O::None),
        0x59 => ("LD E,C", O::None),
        0x5A => ("LD E,D", O::None),
        0x5B => ("LD E,E", O::None),
        0x5C => ("LD E,H", O::None),
        0x5D => ("LD E,L", O::None),
        0x5E => ("LD E,(HL)", O::None),
        0x5F => ("LD E,A", O::None),
        0x60 => ("LD H,B", O::None),
        0x61 => ("LD H,C", O::None),
        0x62 => ("LD H,D", O::None),
        0x63 => ("LD H,E", O::None),
        0x64 => ("LD H,H", O::None),
        0x65 => ("LD H,L", O::None),
        0x66 => ("LD H,(HL)", O::None),
        0x67 => ("LD H,A", O::None),
        0x68 => ("LD L,B", O::None),
        0x69 => ("LD L,C", O::None),
        0x6A => ("LD L,D", O::None),
        0x6B => ("LD L,E", O::None),
        0x6C => ("LD L,H", O::None),
        0x6D => ("LD L,L", O::None),
        0x6E => ("LD L,(HL)", O::None),
        0x6F => ("LD L,A", O::None),
        0x70 => ("LD (HL),B", O::None),
        0x71 => ("LD (HL),C", O::None),
        0x72 => ("LD (HL),D", O::None),
        0x73 => ("LD (HL),E", O::None),
        0x74 => ("LD (HL),H", O::None),
        0x75 => ("LD (HL),L", O::None),
        0x76 => ("HALT", O::None),
        0x77 => ("LD (HL),A", O::None),
        0x78 => ("LD A,B", O::None),
        0x79 => ("LD A,C", O::None),
        0x7A => ("LD A,D", O::None),
        0x7B => ("LD A,E", O::None),
        0x7C => ("LD A,H", O::None),
        0x7D => ("LD A,L", O::None),
        0x7E => ("LD A,(HL)", O::None),
        0x7F => ("LD A,A", O::None),
        // x8-xB: ALU r
        0x80 => ("ADD A,B", O::None),
        0x81 => ("ADD A,C", O::None),
        0x82 => ("ADD A,D", O::None),
        0x83 => ("ADD A,E", O::None),
        0x84 => ("ADD A,H", O::None),
        0x85 => ("ADD A,L", O::None),
        0x86 => ("ADD A,(HL)", O::None),
        0x87 => ("ADD A,A", O::None),
        0x88 => ("ADC A,B", O::None),
        0x89 => ("ADC A,C", O::None),
        0x8A => ("ADC A,D", O::None),
        0x8B => ("ADC A,E", O::None),
        0x8C => ("ADC A,H", O::None),
        0x8D => ("ADC A,L", O::None),
        0x8E => ("ADC A,(HL)", O::None),
        0x8F => ("ADC A,A", O::None),
        0x90 => ("SUB B", O::None),
        0x91 => ("SUB C", O::None),
        0x92 => ("SUB D", O::None),
        0x93 => ("SUB E", O::None),
        0x94 => ("SUB H", O::None),
        0x95 => ("SUB L", O::None),
        0x96 => ("SUB (HL)", O::None),
        0x97 => ("SUB A", O::None),
        0x98 => ("SBC A,B", O::None),
        0x99 => ("SBC A,C", O::None),
        0x9A => ("SBC A,D", O::None),
        0x9B => ("SBC A,E", O::None),
        0x9C => ("SBC A,H", O::None),
        0x9D => ("SBC A,L", O::None),
        0x9E => ("SBC A,(HL)", O::None),
        0x9F => ("SBC A,A", O::None),
        0xA0 => ("AND B", O::None),
        0xA1 => ("AND C", O::None),
        0xA2 => ("AND D", O::None),
        0xA3 => ("AND E", O::None),
        0xA4 => ("AND H", O::None),
        0xA5 => ("AND L", O::None),
        0xA6 => ("AND (HL)", O::None),
        0xA7 => ("AND A", O::None),
        0xA8 => ("XOR B", O::None),
        0xA9 => ("XOR C", O::None),
        0xAA => ("XOR D", O::None),
        0xAB => ("XOR E", O::None),
        0xAC => ("XOR H", O::None),
        0xAD => ("XOR L", O::None),
        0xAE => ("XOR (HL)", O::None),
        0xAF => ("XOR A", O::None),
        0xB0 => ("OR B", O::None),
        0xB1 => ("OR C", O::None),
        0xB2 => ("OR D", O::None),
        0xB3 => ("OR E", O::None),
        0xB4 => ("OR H", O::None),
        0xB5 => ("OR L", O::None),
        0xB6 => ("OR (HL)", O::None),
        0xB7 => ("OR A", O::None),
        0xB8 => ("CP B", O::None),
        0xB9 => ("CP C", O::None),
        0xBA => ("CP D", O::None),
        0xBB => ("CP E", O::None),
        0xBC => ("CP H", O::None),
        0xBD => ("CP L", O::None),
        0xBE => ("CP (HL)", O::None),
        0xBF => ("CP A", O::None),
        // xC-xF: control, jumps, stack
        0xC0 => ("RET NZ", O::None),
        0xC1 => ("POP BC", O::None),
        0xC2 => ("JP NZ", O::Imm16),
        0xC3 => ("JP", O::Imm16),
        0xC4 => ("CALL NZ", O::Imm16),
        0xC5 => ("PUSH BC", O::None),
        0xC6 => ("ADD A", O::Imm8),
        0xC7 => ("RST", O::None),
        0xC8 => ("RET Z", O::None),
        0xC9 => ("RET", O::None),
        0xCA => ("JP Z", O::Imm16),
        0xCC => ("CALL Z", O::Imm16),
        0xCD => ("CALL", O::Imm16),
        0xCE => ("ADC A", O::Imm8),
        0xCF => ("RST", O::None),
        0xD0 => ("RET NC", O::None),
        0xD1 => ("POP DE", O::None),
        0xD2 => ("JP NC", O::Imm16),
        0xD4 => ("CALL NC", O::Imm16),
        0xD5 => ("PUSH DE", O::None),
        0xD6 => ("SUB", O::Imm8),
        0xD7 => ("RST", O::None),
        0xD8 => ("RET C", O::None),
        0xD9 => ("RETI", O::None),
        0xDA => ("JP C", O::Imm16),
        0xDC => ("CALL C", O::Imm16),
        0xDE => ("SBC A", O::Imm8),
        0xDF => ("RST", O::None),
        0xE0 => ("LDH (n),A", O::Imm8),
        0xE1 => ("POP HL", O::None),
        0xE2 => ("LD (C),A", O::None),
        0xE5 => ("PUSH HL", O::None),
        0xE6 => ("AND", O::Imm8),
        0xE7 => ("RST", O::None),
        0xE8 => ("ADD SP", O::Imm8),
        0xE9 => ("JP HL", O::None),
        0xEA => ("LD (nn),A", O::Imm16),
        0xEE => ("XOR", O::Imm8),
        0xEF => ("RST", O::None),
        0xF0 => ("LDH A,(n)", O::Imm8),
        0xF1 => ("POP AF", O::None),
        0xF2 => ("LD A,(C)", O::None),
        0xF3 => ("DI", O::None),
        0xF5 => ("PUSH AF", O::None),
        0xF6 => ("OR", O::Imm8),
        0xF7 => ("RST", O::None),
        0xF8 => ("LD HL,SP", O::Imm8),
        0xF9 => ("LD SP,HL", O::None),
        0xFA => ("LD A,(nn)", O::Imm16),
        0xFB => ("EI", O::None),
        0xFE => ("CP", O::Imm8),
        0xFF => ("RST", O::None),
        // Illegal: raw byte, never fabricated.
        _ => ("DB", O::None),
    }
}

/// Decode one `CB`-prefixed opcode to text (always 2 bytes).
fn decode_cb(op: u8) -> String {
    const REGS: [&str; 8] = ["B", "C", "D", "E", "H", "L", "(HL)", "A"];
    let reg = REGS[(op & 7) as usize];
    match op {
        0x00..=0x07 => format!("RLC {reg}"),
        0x08..=0x0F => format!("RRC {reg}"),
        0x10..=0x17 => format!("RL {reg}"),
        0x18..=0x1F => format!("RR {reg}"),
        0x20..=0x27 => format!("SLA {reg}"),
        0x28..=0x2F => format!("SRA {reg}"),
        0x30..=0x37 => format!("SWAP {reg}"),
        0x38..=0x3F => format!("SRL {reg}"),
        0x40..=0x7F => format!("BIT {},{reg}", (op >> 3) & 7),
        0x80..=0xBF => format!("RES {},{reg}", (op >> 3) & 7),
        _ => format!("SET {},{reg}", (op >> 3) & 7),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lengths_cover_all_opcodes() {
        for op in 0..=255u8 {
            let len = instr_len(op);
            assert!((1..=3).contains(&len), "op {op:02X} len {len}");
        }
    }

    #[test]
    fn decode_vectors() {
        let d = decode(0x100, 0x01, 0x34, 0x12);
        assert_eq!((d.text.as_str(), d.len, d.target), ("LD BC $1234", 3, None));
        let d = decode(0x100, 0x18, 0xFE, 0x00);
        assert_eq!(
            (d.text.as_str(), d.len, d.target),
            ("JR $0100", 2, Some(0x100))
        );
        let d = decode(0x100, 0xC3, 0x48, 0x00);
        assert_eq!(
            (d.text.as_str(), d.len, d.target),
            ("JP $0048", 3, Some(0x48))
        );
        let d = decode(0x100, 0xC7, 0x00, 0x00);
        assert_eq!((d.text.as_str(), d.len, d.target), ("RST $00", 1, Some(0)));
        let d = decode(0xCB, 0xCB, 0x7C, 0x00);
        assert_eq!((d.text.as_str(), d.len, d.target), ("BIT 7,H", 2, None));
        let d = decode(0x100, 0xD3, 0x00, 0x00);
        assert_eq!((d.text.as_str(), d.len, d.target), (".DB $D3", 1, None));
        let d = decode(0x100, 0xE0, 0x80, 0x00);
        assert_eq!(
            (d.text.as_str(), d.len, d.target),
            ("LDH ($FF80),A", 2, None)
        );
    }

    #[test]
    fn decode_illegal_set_is_raw() {
        for op in [
            0xD3, 0xDB, 0xDD, 0xE3, 0xE4, 0xEB, 0xEC, 0xED, 0xF4, 0xFC, 0xFD,
        ] {
            let d = decode(0, op, 0, 0);
            assert_eq!(d.len, 1);
            assert_eq!(d.text, format!(".DB ${op:02X}"));
            assert_eq!(d.target, None);
        }
    }
}
