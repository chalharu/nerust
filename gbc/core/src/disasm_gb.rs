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
        Operand::None => name,
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
///
/// Computed from the LR35902 matrix structure rather than a flat
/// 256-arm table: the `LD r,r'` (0x40-0x7F) and `ALU A,r`
/// (0x80-0xBF) blocks are bitfield grids, and the irregular low and
/// high rows decode per column nibble. The golden test locks all 256
/// mappings, so any drift fails loudly.
fn shape(op: u8) -> (String, Operand) {
    use Operand as O;
    const REGS: [&str; 8] = ["B", "C", "D", "E", "H", "L", "(HL)", "A"];
    // LD r,r' grid: HALT at 0x76, everything else mechanical.
    if (0x40..0x80).contains(&op) {
        if op == 0x76 {
            return ("HALT".to_string(), O::None);
        }
        let dst = REGS[((op >> 3) & 7) as usize];
        let src = REGS[(op & 7) as usize];
        return (format!("LD {dst},{src}"), O::None);
    }
    // ALU A,r grid: eight ops times eight registers. Bare
    // mnemonics take a space, `A`-suffixed ones a comma.
    if (0x80..0xC0).contains(&op) {
        const OPS: [&str; 8] = ["ADD A", "ADC A", "SUB", "SBC A", "AND", "XOR", "OR", "CP"];
        let name = OPS[((op >> 3) & 7) as usize];
        let sep = if name.contains(' ') { "," } else { " " };
        let text = format!("{name}{sep}{}", REGS[(op & 7) as usize]);
        return (text, O::None);
    }
    if op < 0x40 {
        low_block(op)
    } else {
        high_block(op)
    }
}

/// Rows 0x0-0x3, decoded per column nibble (four opcodes each).
fn low_block(op: u8) -> (String, Operand) {
    use Operand as O;
    let hi = op >> 4;
    match op & 0x0F {
        0x0 => match hi {
            0 => ("NOP".to_string(), O::None),
            1 => ("STOP".to_string(), O::Imm8),
            2 => ("JR NZ".to_string(), O::Rel8),
            _ => ("JR NC".to_string(), O::Rel8),
        },
        0x1 => {
            const RR: [&str; 4] = ["BC", "DE", "HL", "SP"];
            (format!("LD {}", RR[hi as usize]), O::Imm16)
        }
        0x2 => {
            const MEM: [&str; 4] = ["LD (BC),A", "LD (DE),A", "LD (HL+),A", "LD (HL-),A"];
            (MEM[hi as usize].to_string(), O::None)
        }
        0x3 => {
            const RR: [&str; 4] = ["BC", "DE", "HL", "SP"];
            (format!("INC {}", RR[hi as usize]), O::None)
        }
        0x4 => {
            const R: [&str; 4] = ["B", "D", "H", "(HL)"];
            (format!("INC {}", R[hi as usize]), O::None)
        }
        0x5 => {
            const R: [&str; 4] = ["B", "D", "H", "(HL)"];
            (format!("DEC {}", R[hi as usize]), O::None)
        }
        0x6 => {
            const R: [&str; 4] = ["B", "D", "H", "(HL)"];
            (format!("LD {}", R[hi as usize]), O::Imm8)
        }
        0x7 => {
            const OP: [&str; 4] = ["RLCA", "RLA", "DAA", "SCF"];
            (OP[hi as usize].to_string(), O::None)
        }
        0x8 => match hi {
            0 => ("LD (nn),SP".to_string(), O::Imm16),
            1 => ("JR".to_string(), O::Rel8),
            2 => ("JR Z".to_string(), O::Rel8),
            _ => ("JR C".to_string(), O::Rel8),
        },
        0x9 => {
            const RR: [&str; 4] = ["BC", "DE", "HL", "SP"];
            (format!("ADD HL,{}", RR[hi as usize]), O::None)
        }
        0xA => {
            const MEM: [&str; 4] = ["LD A,(BC)", "LD A,(DE)", "LD A,(HL+)", "LD A,(HL-)"];
            (MEM[hi as usize].to_string(), O::None)
        }
        0xB => {
            const RR: [&str; 4] = ["BC", "DE", "HL", "SP"];
            (format!("DEC {}", RR[hi as usize]), O::None)
        }
        0xC => {
            const R: [&str; 4] = ["C", "E", "L", "A"];
            (format!("INC {}", R[hi as usize]), O::None)
        }
        0xD => {
            const R: [&str; 4] = ["C", "E", "L", "A"];
            (format!("DEC {}", R[hi as usize]), O::None)
        }
        0xE => {
            const R: [&str; 4] = ["C", "E", "L", "A"];
            (format!("LD {}", R[hi as usize]), O::Imm8)
        }
        _ => {
            const OP: [&str; 4] = ["RRCA", "RRA", "CPL", "CCF"];
            (OP[hi as usize].to_string(), O::None)
        }
    }
}

/// Rows 0xC-0xF, decoded per column nibble. Gaps decode as raw
/// bytes, mirroring the old fallthrough (verified by the golden
/// test, including every illegal opcode).
fn high_block(op: u8) -> (String, Operand) {
    use Operand as O;
    // hi selects the row: C, D, E, F.
    let hi = (op >> 4) - 0xC;
    match op & 0x0F {
        0x0 => match hi {
            0 => ("RET NZ".to_string(), O::None),
            1 => ("RET NC".to_string(), O::None),
            2 => ("LDH (n),A".to_string(), O::Imm8),
            _ => ("LDH A,(n)".to_string(), O::Imm8),
        },
        0x1 => {
            const RP: [&str; 4] = ["BC", "DE", "HL", "AF"];
            (format!("POP {}", RP[hi as usize]), O::None)
        }
        0x2 => match hi {
            0 => ("JP NZ".to_string(), O::Imm16),
            1 => ("JP NC".to_string(), O::Imm16),
            2 => ("LD (C),A".to_string(), O::None),
            _ => ("LD A,(C)".to_string(), O::None),
        },
        0x3 => match hi {
            0 => ("JP".to_string(), O::Imm16),
            3 => ("DI".to_string(), O::None),
            _ => ("DB".to_string(), O::None),
        },
        0x4 => match hi {
            0 => ("CALL NZ".to_string(), O::Imm16),
            1 => ("CALL NC".to_string(), O::Imm16),
            _ => ("DB".to_string(), O::None),
        },
        0x5 => {
            const RP: [&str; 4] = ["BC", "DE", "HL", "AF"];
            (format!("PUSH {}", RP[hi as usize]), O::None)
        }
        0x6 => {
            const OP: [&str; 4] = ["ADD A", "SUB", "AND", "OR"];
            (OP[hi as usize].to_string(), O::Imm8)
        }
        0x7 => ("RST".to_string(), O::None),
        0x8 => match hi {
            0 => ("RET Z".to_string(), O::None),
            1 => ("RET C".to_string(), O::None),
            2 => ("ADD SP".to_string(), O::Imm8),
            _ => ("LD HL,SP".to_string(), O::Imm8),
        },
        0x9 => match hi {
            0 => ("RET".to_string(), O::None),
            1 => ("RETI".to_string(), O::None),
            2 => ("JP HL".to_string(), O::None),
            _ => ("LD SP,HL".to_string(), O::None),
        },
        0xA => match hi {
            0 => ("JP Z".to_string(), O::Imm16),
            1 => ("JP C".to_string(), O::Imm16),
            2 => ("LD (nn),A".to_string(), O::Imm16),
            _ => ("LD A,(nn)".to_string(), O::Imm16),
        },
        0xB => {
            if hi == 3 {
                ("EI".to_string(), O::None)
            } else {
                ("DB".to_string(), O::None)
            }
        }
        0xC => match hi {
            0 => ("CALL Z".to_string(), O::Imm16),
            1 => ("CALL C".to_string(), O::Imm16),
            _ => ("DB".to_string(), O::None),
        },
        0xD => {
            if hi == 0 {
                ("CALL".to_string(), O::Imm16)
            } else {
                ("DB".to_string(), O::None)
            }
        }
        0xE => {
            const OP: [&str; 4] = ["ADC A", "SBC A", "XOR", "CP"];
            (OP[hi as usize].to_string(), O::Imm8)
        }
        _ => ("RST".to_string(), O::None),
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
    fn decode_sweep_never_panics_with_sane_lengths() {
        // Exhaustive sweep over the primary table plus the CB
        // sub-table: every pattern decodes without panic, with a
        // non-empty mnemonic and a sane length.
        for op in 0..=255u32 {
            let d = decode(0xC000, op as u8, 0, 0);
            assert!(!d.text.is_empty(), "empty text for {op:02X}");
            assert!((1..=3).contains(&d.len), "bad length for {op:02X}");
            if op == 0xCB {
                for b1 in 0..=255u32 {
                    let c = decode(0xC000, 0xCB, b1 as u8, 0);
                    assert!(!c.text.is_empty(), "empty CB text for {b1:02X}");
                }
            }
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

    /// Full-table behavior lock for `shape()`: every primary opcode
    /// maps exactly as the golden file records (illegal opcodes map
    /// to the raw-byte fallback). Structural refactors must keep
    /// this green; the golden file itself is data, not code.
    #[test]
    fn shape_matches_golden_table() {
        for line in include_str!("disasm_gb_golden.txt").lines() {
            let mut parts = line.split('\t');
            let op = u8::from_str_radix(parts.next().unwrap(), 16).unwrap();
            let mnemonic = parts.next().unwrap();
            let mode = parts.next().unwrap();
            let (actual_mnemonic, actual_mode) = shape(op);
            if mode == "<FALLTHROUGH>" {
                assert_eq!(
                    actual_mnemonic.as_str(),
                    "DB",
                    "mnemonic for opcode {op:02X}"
                );
                assert_eq!(
                    format!("{:?}", actual_mode),
                    "None",
                    "mode for opcode {op:02X}"
                );
            } else {
                assert_eq!(
                    actual_mnemonic.as_str(),
                    mnemonic,
                    "mnemonic for opcode {op:02X}"
                );
                assert_eq!(
                    format!("{:?}", actual_mode),
                    mode,
                    "mode for opcode {op:02X}"
                );
            }
        }
    }
}
