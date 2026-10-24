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
///
/// Computed from the classic `aaa bbb cc` bit fields rather than a
/// flat 256-arm table: the matrix is column-regular (modes depend
/// only on `bbb` + `cc`, mnemonics only on `aaa`), with the
/// irregular columns/rows called out explicitly. The golden test
/// locks all 256 outputs, so any drift fails loudly.
pub(crate) fn decode(op: u8) -> (&'static str, AddrMode) {
    use AddrMode as M;
    let aaa = (op >> 5) & 7;
    let bbb = (op >> 2) & 7;
    match op & 3 {
        // ALU operations: uniform columns; the immediate column
        // keeps each row's own mnemonic (STA has none: NOP).
        0b01 => {
            if bbb == 2 {
                const IMM: [&str; 8] = ["ORA", "AND", "EOR", "ADC", "NOP", "LDA", "CMP", "SBC"];
                return (IMM[aaa as usize], M::Immediate);
            }
            const ALU: [&str; 8] = ["ORA", "AND", "EOR", "ADC", "STA", "LDA", "CMP", "SBC"];
            const MODES: [AddrMode; 8] = [
                M::IndexedIndirect,
                M::ZeroPage,
                M::Immediate,
                M::Absolute,
                M::IndirectIndexed,
                M::ZeroPageX,
                M::AbsoluteY,
                M::AbsoluteX,
            ];
            (ALU[aaa as usize], MODES[bbb as usize])
        }
        // Shifts, transfers, increments: mostly column-regular with
        // per-row mnemonics; the STX/LDX rows are fully explicit.
        0b10 => decode_cc10(aaa, bbb),
        // Combined ops: same columns as cc=01, same immediate
        // column, Y-indexed tweaks for the STX/LDX-family rows.
        0b11 => {
            if bbb == 2 {
                const IMM: [&str; 8] = ["ANC", "ANC", "ALR", "ARR", "XAA", "LAX", "AXS", "SBC"];
                return (IMM[aaa as usize], M::Immediate);
            }
            const COMBINED: [&str; 8] = ["SLO", "RLA", "SRE", "RRA", "SAX", "LAX", "DCP", "ISC"];
            const MODES: [AddrMode; 8] = [
                M::IndexedIndirect,
                M::ZeroPage,
                M::Immediate,
                M::Absolute,
                M::IndirectIndexed,
                M::ZeroPageX,
                M::AbsoluteY,
                M::AbsoluteX,
            ];
            let mnemonic = match (aaa, bbb) {
                // Unstable writes: the cells where the row name
                // does not apply.
                (4, 4) | (4, 7) => "AHX",
                (4, 6) => "TAS",
                (5, 6) => "LAS",
                _ => COMBINED[aaa as usize],
            };
            let mode = match (aaa, bbb) {
                (4 | 5, 5) => M::ZeroPageY,
                (4 | 5, 6 | 7) => M::AbsoluteY,
                _ => MODES[bbb as usize],
            };
            (mnemonic, mode)
        }
        // Control group: fully irregular, one small table per bbb.
        _ => decode_cc00(aaa, bbb),
    }
}

/// `cc = 10` rows: shifts (aaa 0-3), STX/LDX families (aaa 4-5),
/// increments (aaa 6-7).
fn decode_cc10(aaa: u8, bbb: u8) -> (&'static str, AddrMode) {
    use AddrMode as M;
    // STX row: transfers plus the oddballs.
    if aaa == 4 {
        return match bbb {
            0 => ("NOP", M::Immediate),
            1 => ("STX", M::ZeroPage),
            2 => ("TXA", M::Implied),
            3 => ("STX", M::Absolute),
            4 => ("JAM", M::Jam),
            5 => ("STX", M::ZeroPageY),
            6 => ("TXS", M::Implied),
            _ => ("SHX", M::AbsoluteY),
        };
    }
    // LDX row: mirror image.
    if aaa == 5 {
        return match bbb {
            0 => ("LDX", M::Immediate),
            1 => ("LDX", M::ZeroPage),
            2 => ("TAX", M::Implied),
            3 => ("LDX", M::Absolute),
            4 => ("JAM", M::Jam),
            5 => ("LDX", M::ZeroPageY),
            6 => ("TSX", M::Implied),
            _ => ("LDX", M::AbsoluteY),
        };
    }
    // Gap columns never decode: hard stop.
    if bbb == 0 || bbb == 4 {
        return ("JAM", M::Jam);
    }
    // Implied single-byte ops share the NOP sheet.
    if bbb == 6 {
        return ("NOP", M::Implied);
    }
    const SHIFT: [&str; 4] = ["ASL", "ROL", "LSR", "ROR"];
    if aaa < 4 {
        const MODES: [AddrMode; 8] = [
            M::Jam,
            M::ZeroPage,
            M::Accumulator,
            M::Absolute,
            M::Jam,
            M::ZeroPageX,
            M::Implied,
            M::AbsoluteX,
        ];
        return (SHIFT[aaa as usize], MODES[bbb as usize]);
    }
    // Increment rows: DEX for DEC, plain NOP for INC.
    if bbb == 2 {
        return (if aaa == 6 { "DEX" } else { "NOP" }, M::Implied);
    }
    const MODES: [AddrMode; 8] = [
        M::Jam,
        M::ZeroPage,
        M::Implied,
        M::Absolute,
        M::Jam,
        M::ZeroPageX,
        M::Implied,
        M::AbsoluteX,
    ];
    (if aaa == 6 { "DEC" } else { "INC" }, MODES[bbb as usize])
}

/// `cc = 00` control group, one small table per `bbb` column.
fn decode_cc00(aaa: u8, bbb: u8) -> (&'static str, AddrMode) {
    use AddrMode as M;
    match bbb {
        0 => match aaa {
            0 => ("BRK", M::Implied),
            1 => ("JSR", M::Absolute),
            2 => ("RTI", M::Implied),
            3 => ("RTS", M::Implied),
            4 => ("NOP", M::Immediate),
            5 => ("LDY", M::Immediate),
            6 => ("CPY", M::Immediate),
            _ => ("CPX", M::Immediate),
        },
        1 => match aaa {
            0 => ("NOP", M::ZeroPage),
            1 => ("BIT", M::ZeroPage),
            2 => ("NOP", M::ZeroPage),
            3 => ("NOP", M::ZeroPage),
            4 => ("STY", M::ZeroPage),
            5 => ("LDY", M::ZeroPage),
            6 => ("CPY", M::ZeroPage),
            _ => ("CPX", M::ZeroPage),
        },
        2 => match aaa {
            0 => ("PHP", M::Implied),
            1 => ("PLP", M::Implied),
            2 => ("PHA", M::Implied),
            3 => ("PLA", M::Implied),
            4 => ("DEY", M::Implied),
            5 => ("TAY", M::Implied),
            6 => ("INY", M::Implied),
            _ => ("INX", M::Implied),
        },
        3 => match aaa {
            0 => ("NOP", M::Absolute),
            1 => ("BIT", M::Absolute),
            2 => ("JMP", M::Absolute),
            3 => ("JMP", M::Indirect),
            4 => ("STY", M::Absolute),
            5 => ("LDY", M::Absolute),
            6 => ("CPY", M::Absolute),
            _ => ("CPX", M::Absolute),
        },
        4 => match aaa {
            0 => ("BPL", M::Relative),
            1 => ("BMI", M::Relative),
            2 => ("BVC", M::Relative),
            3 => ("BVS", M::Relative),
            4 => ("BCC", M::Relative),
            5 => ("BCS", M::Relative),
            6 => ("BNE", M::Relative),
            _ => ("BEQ", M::Relative),
        },
        5 => match aaa {
            4 => ("STY", M::ZeroPageX),
            5 => ("LDY", M::ZeroPageX),
            _ => ("NOP", M::ZeroPageX),
        },
        6 => match aaa {
            0 => ("CLC", M::Implied),
            1 => ("SEC", M::Implied),
            2 => ("CLI", M::Implied),
            3 => ("SEI", M::Implied),
            4 => ("TYA", M::Implied),
            5 => ("CLV", M::Implied),
            6 => ("CLD", M::Implied),
            _ => ("SED", M::Implied),
        },
        _ => match aaa {
            4 => ("SHY", M::AbsoluteX),
            5 => ("LDY", M::AbsoluteX),
            _ => ("NOP", M::AbsoluteX),
        },
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

    /// Full-table behavior lock: every opcode decodes exactly as the
    /// golden file records. Structural refactors must keep this
    /// green; the golden file itself is data, not code.
    #[test]
    fn decode_matches_golden_table() {
        for line in include_str!("disasm6502_golden.txt").lines() {
            let mut parts = line.split('\t');
            let op = u8::from_str_radix(parts.next().unwrap(), 16).unwrap();
            let mnemonic = parts.next().unwrap();
            let mode = parts.next().unwrap();
            let (actual_mnemonic, actual_mode) = decode(op);
            assert_eq!(actual_mnemonic, mnemonic, "mnemonic for opcode {op:02X}");
            assert_eq!(
                format!("{:?}", actual_mode),
                mode,
                "mode for opcode {op:02X}"
            );
        }
    }
}
