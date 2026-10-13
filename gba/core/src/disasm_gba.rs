//! GBA disassembler (ARM + Thumb) for the debugger.
//!
//! Pure decode over already-fetched words: no bus access, no core
//! state. The caller (`debugger.rs`) selects the set from the CPSR
//! T-bit, fetches the word through the read-only observer, and owns
//! the cursor advance. Unknown encodings decode as raw data
//! directives (`.HWORD`/`.WORD`), never fabricated instructions.
//!
//! Branch targets use pipeline-adjusted bases (Thumb: `addr + 4`,
//! ARM: `addr + 8`), matching hardware PC semantics. Only plain
//! code addresses become follow targets; register-indirect (`BX`),
//! literal-pool (`LDR [PC]`), and data forms stay `None`.

/// One decoded row: text, follow target (if a plain code address),
/// and instruction length in bytes.
pub struct Decoded {
    pub text: String,
    pub target: Option<u32>,
    pub len: u8,
}

const THUMB_COND: [&str; 14] = [
    "EQ", "NE", "CS", "CC", "MI", "PL", "VS", "VC", "HI", "LS", "GE", "LT", "GT", "LE",
];

const ARM_COND: [&str; 15] = [
    "EQ", "NE", "CS", "CC", "MI", "PL", "VS", "VC", "HI", "LS", "GE", "LT", "GT", "LE", "",
];

const THUMB_ALU: [&str; 16] = [
    "AND", "EOR", "LSL", "LSR", "ASR", "ADC", "SBC", "ROR", "TST", "NEG", "CMP", "CMN", "ORR",
    "MUL", "BIC", "MVN",
];

const ARM_DP: [&str; 16] = [
    "AND", "EOR", "SUB", "RSB", "ADD", "ADC", "SBC", "RSC", "TST", "TEQ", "CMP", "CMN", "ORR",
    "MOV", "BIC", "MVN",
];

fn reg(n: u32) -> String {
    format!("r{n}")
}

/// Decode one 16-bit Thumb halfword at `addr`.
pub fn decode_thumb(addr: u32, word: u16) -> Decoded {
    let w = u32::from(word);
    // First match wins: the groups below partition the halfword
    // space exactly like the old flat chain, so behavior is
    // identical. Anything unmatched stays a raw halfword: the
    // debugger never fabricates a cross-halfword instruction.
    let found = thumb_branch(addr, w)
        .or_else(|| thumb_alu(w))
        .or_else(|| thumb_mem(w))
        .or_else(|| thumb_low(w));
    match found {
        Some((text, target)) => Decoded {
            text,
            target,
            len: 2,
        },
        None => Decoded {
            text: format!(".HWORD ${word:04X}"),
            target: None,
            len: 2,
        },
    }
}

/// Branches and exchanges: conditional (0xD000-0xDDFF),
/// unconditional (0xE000-0xE800), BX/BLX (0x4700-0x47FF).
/// Reserved 0xDE__ stays raw (THUMB_COND holds 14 entries).
fn thumb_branch(addr: u32, w: u32) -> Option<(String, Option<u32>)> {
    // Conditional branch (0xD000-0xDDFF; 0xDE__ is a reserved
    // condition and 0xDFxx is SWI).
    if (0xD000..0xDE00).contains(&w) {
        let cond = THUMB_COND[((w >> 8) & 0xF) as usize];
        let offset = ((w & 0xFF) as i8 as i32 * 2).wrapping_add(addr.wrapping_add(4) as i32);
        return Some((
            format!("B{cond} ${:08X}", offset as u32),
            Some(offset as u32),
        ));
    }
    if (0xDE00..0xDF00).contains(&w) {
        // Reserved condition (NV): raw halfword, never fabricated
        // (indexing THUMB_COND here would panic: it holds 14).
        return None;
    }
    // Unconditional branch: signed 11-bit field, doubled, from base+4.
    if (0xE000..0xE800).contains(&w) {
        let raw = w & 0x7FF;
        let field = ((raw << 21) as i32) >> 21;
        let offset = field
            .wrapping_mul(2)
            .wrapping_add(addr.wrapping_add(4) as i32);
        return Some((format!("B ${:08X}", offset as u32), Some(offset as u32)));
    }
    // Branch/exchange (0x4700-0x47FF; H set selects BLX register).
    if (0x4700..0x4800).contains(&w) {
        let rm = (w >> 3) & 0xF;
        let text = if w & 0x80 != 0 {
            format!("BLX {}", reg(rm))
        } else {
            format!("BX {}", reg(rm))
        };
        return Some((text, None));
    }
    None
}

/// ALU forms: hi-register (0x4400-0x46FF), ALU ops
/// (0x4000-0x43FF), move/compare/add/sub immediate
/// (0x2000-0x3FFF). None carries a code target.
fn thumb_alu(w: u32) -> Option<(String, Option<u32>)> {
    // Hi-register ALU (0x4400-0x46FF): ADD/CMP/MOV.
    if (0x4400..0x4700).contains(&w) {
        let op = ["ADD", "CMP", "MOV"][((w >> 8) & 3) as usize];
        let rd = (w & 7) | ((w >> 4) & 8);
        let rs = (w >> 3) & 0xF;
        return Some((format!("{op} {}, {}", reg(rd), reg(rs)), None));
    }
    // ALU ops (0x4000-0x43FF).
    if (0x4000..0x4400).contains(&w) {
        let op = THUMB_ALU[((w >> 6) & 0xF) as usize];
        let rs = (w >> 3) & 7;
        let rd = w & 7;
        return Some((format!("{op} {}, {}", reg(rd), reg(rs)), None));
    }
    // Move/compare/add/sub immediate (0x2000-0x3FFF).
    if (0x2000..0x4000).contains(&w) {
        let op = ["MOV", "CMP", "ADD", "SUB"][((w >> 11) & 3) as usize];
        let rd = (w >> 8) & 7;
        return Some((format!("{op} {}, #${:02X}", reg(rd), w & 0xFF), None));
    }
    None
}

/// Memory forms: PC-relative/absolute loads, register and
/// immediate offsets, ADD PC/SP, ADD SP/PUSH/POP, and multiple
/// load/store. Pool addresses are data, never code targets.
fn thumb_mem(w: u32) -> Option<(String, Option<u32>)> {
    // PC-relative load (0x4800-0x4FFF): pool address is data, not code.
    if (0x4800..0x5000).contains(&w) {
        let rd = (w >> 8) & 7;
        return Some((
            format!("LDR {}, [PC, #${:02X}]", reg(rd), (w & 0xFF) * 4),
            None,
        ));
    }
    // Register-offset load/store (0x5000-0x5FFF).
    if (0x5000..0x6000).contains(&w) {
        let op =
            ["STR", "STRH", "STRB", "LDSB", "LDR", "LDRH", "LDRB", "LDSH"][((w >> 9) & 7) as usize];
        return Some((
            format!(
                "{op} {}, [{}, {}]",
                reg(w & 7),
                reg((w >> 3) & 7),
                reg((w >> 6) & 7)
            ),
            None,
        ));
    }
    if let Some(found) = thumb_mem_offset(w) {
        return Some(found);
    }
    if let Some(found) = thumb_mem_stack(w) {
        return Some(found);
    }
    // Multiple load/store (0xC000-0xCFFF).
    if (0xC000..0xD000).contains(&w) {
        let op = if w & 0x0800 != 0 { "LDMIA" } else { "STMIA" };
        return Some((
            format!(
                "{op} {}!, {{{}}}",
                reg((w >> 8) & 7),
                reg_list(w, 8).join(", ")
            ),
            None,
        ));
    }
    None
}

/// Immediate-offset load/store, byte/halfword variants,
/// SP-relative, and ADD to PC/SP.
fn thumb_mem_offset(w: u32) -> Option<(String, Option<u32>)> {
    // Immediate-offset load/store, byte/halfword variants, SP-relative.
    if (0x6000..0xA000).contains(&w) {
        let is_load = w & 0x0800 != 0;
        let (op, scale) = match (w >> 12) & 7 {
            0x6 => (if is_load { "LDR" } else { "STR" }, 4),
            0x7 => (if is_load { "LDRB" } else { "STRB" }, 1),
            0x8 => (if is_load { "LDRH" } else { "STRH" }, 2),
            _ => (if is_load { "LDR" } else { "STR" }, 4),
        };
        let base = if w >= 0x9000 {
            "SP".to_string()
        } else {
            reg((w >> 3) & 7)
        };
        let offset = if w >= 0x9000 {
            (w & 0xFF) * 4
        } else {
            ((w >> 6) & 0x1F) * scale
        };
        return Some((
            format!("{op} {}, [{base}, #${offset:02X}]", reg(w & 7)),
            None,
        ));
    }
    // ADD to PC/SP (0xA000-0xAFFF).
    if (0xA000..0xB000).contains(&w) {
        let dst = if w & 0x0800 != 0 { "SP" } else { "PC" };
        return Some((
            format!(
                "ADD {}, {}, #${:02X}",
                reg((w >> 8) & 7),
                dst,
                (w & 0xFF) * 4
            ),
            None,
        ));
    }
    None
}

/// ADD SP / PUSH / POP. PUSH is 1011 010L, POP 1011 110L:
/// bits 10-9 read 0b10 and bit 11 selects store/load.
fn thumb_mem_stack(w: u32) -> Option<(String, Option<u32>)> {
    if !(0xB000..0xBE00).contains(&w) {
        return None;
    }
    if w < 0xB100 {
        let off = (w & 0x7F) * 4;
        let text = if w & 0x80 != 0 {
            format!("ADD SP, #-${off:02X}")
        } else {
            format!("ADD SP, #${off:02X}")
        };
        return Some((text, None));
    }
    if (w >> 9) & 3 == 2 {
        let store = w & 0x0800 == 0;
        let mut list = reg_list(w, 8);
        if w & 0x0100 != 0 {
            list.push(if store { "LR" } else { "PC" }.to_string());
        }
        return Some((
            format!(
                "{} {{{}}}",
                if store { "PUSH" } else { "POP" },
                list.join(", ")
            ),
            None,
        ));
    }
    None
}

/// Low-halfword forms: shifts, small-immediate add/sub, and SWI.
fn thumb_low(w: u32) -> Option<(String, Option<u32>)> {
    // Shift/add/sub small immediates and SWI.
    if w < 0x2000 {
        let rd = w & 7;
        let rs = (w >> 3) & 7;
        if w < 0x0800 {
            return Some((
                format!("LSL {}, {}, #${:02X}", reg(rd), reg(rs), (w >> 6) & 0x1F),
                None,
            ));
        }
        if w < 0x1000 {
            return Some((
                format!("LSR {}, {}, #${:02X}", reg(rd), reg(rs), (w >> 6) & 0x1F),
                None,
            ));
        }
        if w < 0x1800 {
            return Some((
                format!("ASR {}, {}, #${:02X}", reg(rd), reg(rs), (w >> 6) & 0x1F),
                None,
            ));
        }
        let op = if w & 0x0400 != 0 { "SUB" } else { "ADD" };
        if w & 0x0200 != 0 {
            return Some((
                format!("{op} {}, {}, #${:02X}", reg(rd), reg(rs), (w >> 6) & 7),
                None,
            ));
        }
        return Some((
            format!("{op} {}, {}, {}", reg(rd), reg(rs), reg((w >> 6) & 7)),
            None,
        ));
    }
    if (0xDF00..0xE000).contains(&w) {
        return Some((format!("SWI #${:02X}", w & 0xFF), None));
    }
    None
}

fn reg_list(bits: u32, count: u32) -> Vec<String> {
    (0..count)
        .filter(|r| bits & (1 << r) != 0)
        .map(reg)
        .collect()
}

/// Decode one 32-bit ARM word at `addr`. `cond` names the condition
/// field; `NV` (0xF) is unpredictable and stays raw.
pub fn decode_arm(addr: u32, word: u32) -> Decoded {
    let cond_field = (word >> 28) as usize;
    // NV is unpredictable on all real cores: raw, before any table.
    if cond_field == 0xF {
        return Decoded {
            text: format!(".WORD ${word:08X}"),
            target: None,
            len: 4,
        };
    }
    if let Some(branch) = arm_branch(addr, word, cond_field) {
        return branch;
    }
    // First match wins, in the old chain order; anything unmatched
    // stays a raw word, never a fabricated instruction.
    let cond = ARM_COND[cond_field];
    let text = arm_bx(word, cond)
        .or_else(|| arm_multiply(word, cond))
        .or_else(|| arm_data_processing(word, cond))
        .or_else(|| arm_single_transfer(word, cond))
        .or_else(|| arm_block_transfer(word, cond))
        .or_else(|| arm_swi(word, cond))
        .unwrap_or_else(|| format!(".WORD ${word:08X}"));
    Decoded {
        text,
        target: None,
        len: 4,
    }
}

/// B / BL: 24-bit signed offset scaled by 4 from addr + 8.
/// The top nibble is the condition, so mask it before matching.
fn arm_branch(addr: u32, word: u32, cond_field: usize) -> Option<Decoded> {
    let branch = (word >> 24) & 0x0F;
    if branch != 0x0A && branch != 0x0B {
        return None;
    }
    let raw = word & 0xFFFFFF;
    let offset = (((raw << 8) as i32) >> 6).wrapping_add(addr.wrapping_add(8) as i32);
    let op = if branch == 0x0B { "BL" } else { "B" };
    let cond = ARM_COND[cond_field];
    Some(Decoded {
        text: format!("{op}{cond} ${:08X}", offset as u32),
        target: Some(offset as u32),
        len: 4,
    })
}

/// BX / BLX Rm (bit 4 selects the v5 register form). The mask
/// accepts both tails: BX ends in 0x1, BLX in 0x3.
fn arm_bx(word: u32, cond: &str) -> Option<String> {
    let tail = word & 0x0FFFFFF0;
    if tail != 0x012FFF10 && tail != 0x012FFF30 {
        return None;
    }
    let rm = word & 0xF;
    Some(if word & 0xF0 == 0x30 {
        format!("BLX{cond} {}", reg(rm))
    } else {
        format!("BX{cond} {}", reg(rm))
    })
}

/// Multiply: MUL / MLA (+S). Bits 27-22 read zero and the shifter
/// tail reads 1001, which a data-processing op with a register LSL
/// shift can also match — so MUL additionally requires Rn == 0
/// (SBZ) while MLA (A set) takes any Rn.
fn arm_multiply(word: u32, cond: &str) -> Option<String> {
    if (word >> 26) & 3 != 0
        || word & 0x90 != 0x90
        || word & 0x0FC00000 != 0
        || (word & 0x00200000 == 0 && word & 0x0000F000 != 0)
    {
        return None;
    }
    let rd = (word >> 16) & 0xF;
    let rs = (word >> 8) & 0xF;
    let rm = word & 0xF;
    let s = if word & 0x100000 != 0 { "S" } else { "" };
    Some(if word & 0x200000 != 0 {
        let rn = (word >> 12) & 0xF;
        format!(
            "MLA{cond}{s} {}, {}, {}, {}",
            reg(rd),
            reg(rm),
            reg(rs),
            reg(rn)
        )
    } else {
        format!("MUL{cond}{s} {}, {}, {}", reg(rd), reg(rm), reg(rs))
    })
}

/// Data processing: opcode + S + Rn + Rd + shifter operand.
/// Test/compare ops have no Rd; MOV/MVN have no Rn.
fn arm_data_processing(word: u32, cond: &str) -> Option<String> {
    if (word >> 26) & 3 != 0 {
        return None;
    }
    let op = ARM_DP[((word >> 21) & 0xF) as usize];
    let s = if word & 0x100000 != 0 { "S" } else { "" };
    let rn = (word >> 16) & 0xF;
    let rd = (word >> 12) & 0xF;
    let operand = shifter_operand(word);
    let opcode = (word >> 21) & 0xF;
    Some(if (0x8..0xC).contains(&opcode) {
        format!("{op}{cond}{s} {}, {operand}", reg(rn))
    } else if opcode == 0xD || opcode == 0xF {
        format!("{op}{cond}{s} {}, {operand}", reg(rd))
    } else {
        format!("{op}{cond}{s} {}, {}, {operand}", reg(rd), reg(rn))
    })
}

/// Single data transfer: LDR/STR (+B). Bit 21 is writeback
/// pre-indexed (!) but user-mode (T) post-indexed.
fn arm_single_transfer(word: u32, cond: &str) -> Option<String> {
    if (word >> 26) & 3 != 1 {
        return None;
    }
    let op = if word & 0x100000 != 0 { "LDR" } else { "STR" };
    let b = if word & 0x400000 != 0 { "B" } else { "" };
    let rd = (word >> 12) & 0xF;
    let rn = (word >> 16) & 0xF;
    let negative = word & 0x800000 == 0;
    let offset = if word & 0x2000000 == 0 {
        if negative {
            format!("#-${:03X}", word & 0xFFF)
        } else {
            format!("#${:03X}", word & 0xFFF)
        }
    } else {
        // Bit 25 means opposite things in the two users of the
        // barrel shifter: for data processing I=1 is an immediate,
        // but for loads/stores I=1 is a *register* offset. The
        // register-only formatter keeps this path honest; sharing
        // `shifter_operand` here printed immediates.
        let reg_off = shifter_register_operand(word);
        if negative {
            format!("-{reg_off}")
        } else {
            reg_off
        }
    };
    Some(if word & 0x1000000 != 0 {
        let wb = if word & 0x200000 != 0 { "!" } else { "" };
        format!("{op}{cond}{b} {}, [{}, {offset}]{wb}", reg(rd), reg(rn))
    } else {
        let t = if word & 0x200000 != 0 { "T" } else { "" };
        format!("{op}{cond}{b}{t} {}, [{}, {offset}]", reg(rd), reg(rn))
    })
}

/// Block transfer: LDM/STM + addressing mode + register list.
fn arm_block_transfer(word: u32, cond: &str) -> Option<String> {
    if (word >> 26) & 3 != 2 {
        return None;
    }
    let op = if word & 0x100000 != 0 { "LDM" } else { "STM" };
    let mode = ["ED", "EA", "FD", "FA"][((word >> 23) & 3) as usize];
    let rn = (word >> 16) & 0xF;
    let wb = if word & 0x200000 != 0 { "!" } else { "" };
    let s = if word & 0x400000 != 0 { "^" } else { "" };
    Some(format!(
        "{op}{cond}{mode} {}({wb}), {{{}}}{s}",
        reg(rn),
        reg_list(word, 16).join(", ")
    ))
}

/// SWI immediate; anything else stays a raw word.
fn arm_swi(word: u32, cond: &str) -> Option<String> {
    if (word >> 24) & 0x0F != 0x0F {
        return None;
    }
    Some(format!("SWI{cond} #${:06X}", word & 0xFFFFFF))
}

fn shifter_operand(word: u32) -> String {
    if word & 0x2000000 != 0 {
        // Rotated 8-bit immediate.
        let rot = ((word >> 8) & 0xF) * 2;
        let imm = word & 0xFF;
        if rot == 0 {
            format!("#${imm:02X}")
        } else {
            format!("#${imm:02X}, ROR #{rot}")
        }
    } else {
        shifter_register_operand(word)
    }
}

/// Register-shifted operand (loads/stores with I=1 share this;
/// data processing reaches it through [`shifter_operand`]).
fn shifter_register_operand(word: u32) -> String {
    let rm = word & 0xF;
    let shift = (word >> 5) & 3;
    let name = ["LSL", "LSR", "ASR", "ROR"][shift as usize];
    if word & 0x10 == 0 {
        // Immediate shift; amount 0 has shifted meanings.
        let amount = (word >> 7) & 0x1F;
        match (shift, amount) {
            (0, 0) => reg(rm),
            (1, 0) | (2, 0) => format!("{}, {name} #$20", reg(rm)),
            (3, 0) => format!("{}, RRX", reg(rm)),
            _ => format!("{}, {name} #${amount:02X}", reg(rm)),
        }
    } else {
        // Register shift.
        format!("{}, {name} {}", reg(rm), reg((word >> 8) & 0xF))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thumb_unconditional_branch_targets_pipeline_base() {
        // B $0800000C from 0x08000000: 0xE004 -> +8 from base+4.
        let decoded = decode_thumb(0x08000000, 0xE004);
        assert_eq!(decoded.text, "B $0800000C");
        assert_eq!(decoded.target, Some(0x0800000C));
        assert_eq!(decoded.len, 2);
        // 0xE7FE is the self-loop: offset -2 halves to -4 from base+4.
        let decoded = decode_thumb(0x08000000, 0xE7FE);
        assert_eq!(decoded.text, "B $08000000");
        assert_eq!(decoded.target, Some(0x08000000));
    }

    #[test]
    fn thumb_cond_branch_names_condition() {
        // BEQ with offset 0 from 0x08000010: base +4.
        let decoded = decode_thumb(0x08000010, 0xD000);
        assert_eq!(decoded.text, "BEQ $08000014");
        assert_eq!(decoded.target, Some(0x08000014));
        // BNE -4 from 0x08000010: 0xD1FE -> base -4.
        let decoded = decode_thumb(0x08000010, 0xD1FE);
        assert_eq!(decoded.text, "BNE $08000010");
        assert_eq!(decoded.target, Some(0x08000010));
    }

    #[test]
    fn thumb_data_and_control_forms() {
        assert_eq!(decode_thumb(0, 0x46C0).text, "MOV r8, r8");
        assert_eq!(decode_thumb(0, 0x4770).text, "BX r14");
        assert_eq!(decode_thumb(0, 0x2001).text, "MOV r0, #$01");
        assert_eq!(decode_thumb(0, 0x4908).text, "LDR r1, [PC, #$20]");
        assert_eq!(decode_thumb(0, 0xB520).text, "PUSH {r5, LR}");
        assert_eq!(decode_thumb(0, 0xBD00).text, "POP {PC}");
    }

    #[test]
    fn thumb_swi_and_raw_halves() {
        assert_eq!(decode_thumb(0, 0xDF00).text, "SWI #$00");
        assert_eq!(decode_thumb(0, 0xF000).text, ".HWORD $F000");
        assert_eq!(decode_thumb(0, 0xFFFF).text, ".HWORD $FFFF");
        // Register-indirect forms never navigate.
        assert_eq!(decode_thumb(0, 0x4770).target, None);
        assert_eq!(decode_thumb(0, 0x4908).target, None);
    }

    #[test]
    fn thumb_decode_covers_every_halfword() {
        // Exhaustive sweep: every 16-bit pattern decodes without
        // panic, with a non-empty mnemonic and a sane length. This
        // pins every decode arm by construction (and caught the
        // reserved-condition 0xDE__ out-of-bounds panic).
        for word in 0..=0xFFFFu32 {
            let decoded = decode_thumb(0x0800_0000, word as u16);
            assert!(!decoded.text.is_empty(), "empty text for {word:04X}");
            assert!(
                decoded.len == 2 || decoded.len == 4,
                "bad length for {word:04X}"
            );
        }
        // The reserved condition degrades to a raw halfword.
        assert_eq!(decode_thumb(0, 0xDE00).text, ".HWORD $DE00");
        assert_eq!(decode_thumb(0, 0xDEFF).target, None);
    }

    #[test]
    fn arm_branch_uses_plus_eight_base() {
        // B from 0x08000000 with offset 0: 0xEA000000 -> +8.
        let decoded = decode_arm(0x08000000, 0xEA000000);
        assert_eq!(decoded.text, "B $08000008");
        assert_eq!(decoded.target, Some(0x08000008));
        assert_eq!(decoded.len, 4);
        // BL keeps its own mnemonic.
        let decoded = decode_arm(0x08000000, 0xEB000000);
        assert_eq!(decoded.text, "BL $08000008");
        assert_eq!(decoded.target, Some(0x08000008));
    }

    /// Helper-group coverage with independently derived vectors
    /// (encodings verified against the ARM ISA, not against the old
    /// table): BLX, MLA, negative/register-offset/writeback/T-suffix
    /// loads, Thumb BLX/LDRB/SP-relative/LDMIA, rotated immediates.
    #[test]
    fn arm_and_thumb_helper_forms() {
        // BLX r0: 0xE12FFF30 (v5 register form).
        assert_eq!(decode_arm(0, 0xE12FFF30).text, "BLX r0");
        // MLA r1, r2, r3, r0: A bit set, S clear.
        assert_eq!(decode_arm(0, 0xE0210392).text, "MLA r1, r2, r3, r0");
        // Negative, register-offset, writeback, and T-suffix loads.
        assert_eq!(decode_arm(0, 0xE5110004).text, "LDR r0, [r1, #-$004]");
        assert_eq!(decode_arm(0, 0xE7110002).text, "LDR r0, [r1, -r2]");
        assert_eq!(decode_arm(0, 0xE5310004).text, "LDR r0, [r1, #-$004]!");
        assert_eq!(decode_arm(0, 0xE4310004).text, "LDRT r0, [r1, #-$004]");
        // Rotated immediate: MOV r0, #1 ROR #8.
        assert_eq!(decode_arm(0, 0xE3A00401).text, "MOV r0, #$01, ROR #8");
        // Thumb: BLX r9, byte load, SP-relative store, LDMIA.
        assert_eq!(decode_thumb(0, 0x47C8).text, "BLX r9");
        assert_eq!(decode_thumb(0, 0x7801).text, "LDRB r1, [r0, #$00]");
        assert_eq!(decode_thumb(0, 0x9001).text, "STR r1, [SP, #$04]");
        assert_eq!(decode_thumb(0, 0xC801).text, "LDMIA r0!, {r0}");
    }

    #[test]
    fn arm_data_and_control_forms() {
        // MOV r0, #0: 0xE3A00000.
        let decoded = decode_arm(0, 0xE3A00000);
        assert_eq!(decoded.text, "MOV r0, #$00");
        assert_eq!(decoded.target, None);
        // BX r14: 0xE12FFF1E.
        assert_eq!(decode_arm(0, 0xE12FFF1E).text, "BX r14");
        // SWI #0: 0xEF000000.
        assert_eq!(decode_arm(0, 0xEF000000).text, "SWI #$000000");
        // NV condition stays raw.
        assert_eq!(decode_arm(0, 0xFA000000).text, ".WORD $FA000000");
        // LDR r0, [r1, #$008]: 0xE5910008.
        assert_eq!(decode_arm(0, 0xE5910008).text, "LDR r0, [r1, #$008]");
        // Register-shifted AND is data processing, not multiply:
        // AND r5, r0, r3, LSL r9 keeps its mnemonic.
        assert_eq!(decode_arm(0, 0xE0005993).text, "AND r5, r0, r3, LSL r9");
        // Genuine MUL still decodes: MUL r1, r2, r3 = 0xE0010392.
        assert_eq!(decode_arm(0, 0xE0010392).text, "MUL r1, r2, r3");
    }
}
