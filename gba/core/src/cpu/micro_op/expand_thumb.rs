//! Thumb micro-op expansion: one [`MicroOp`] queue per instruction.
//! Pure decode over snapshots (`regs`); shares only the op types
//! from the parent module, so this file depends downward on nothing.
use super::{
    AluEffect, AluImmOp, BlockEmptyEffect, BlockEndEffect, BlockStartEffect, BlockWord,
    BranchEffect, MemAccess, MicroOp, MulEffect, PcRelRead,
};
use crate::cpu::semantics::{condition_passed, multiplier_cycles};
use crate::cpu_registers::CpuRegisters;

/// Expand a Thumb instruction. `None` = not covered yet (legacy path).
/// `regs` snapshots stack/base pointers and STM store words at
/// queue-fill (pre-instruction state; registers are frozen across the
/// words of one instruction).
pub fn expand_thumb(instr: u16, regs: &CpuRegisters) -> Option<Vec<MicroOp>> {
    // Conditional B (cond < 0xE); 0xDE00/0xDF00 fall through to the
    // UND/SWI traps below.
    if instr >> 12 == 0xD && ((instr >> 8) & 0xF) < 0xE {
        let condition = ((instr >> 8) & 0xF) as u8;
        if !condition_passed(regs.cpsr(), condition) {
            return Some(vec![MicroOp::Internal]);
        }
        let offset = ((instr & 0xFF) as i8 as i32) << 1;
        return Some(vec![
            MicroOp::Internal,
            MicroOp::Internal,
            MicroOp::TakenBranch(BranchEffect {
                offset: offset as u32,
                link: false,
            }),
        ]);
    }
    // B (unconditional).
    if instr >> 11 == 0b11100 {
        let offset = ((instr & 0x7FF) as i32) << 1;
        let offset = (offset << 20) >> 20;
        return Some(vec![
            MicroOp::Internal,
            MicroOp::Internal,
            MicroOp::TakenBranch(BranchEffect {
                offset: offset as u32,
                link: false,
            }),
        ]);
    }
    // Long BL: high half sets LR (1 cycle); low half refills like B.
    if instr >> 11 == 0b11110 {
        let offset = ((instr & 0x7FF) as i32) << 12;
        let offset = (offset << 9) >> 9; // sign extend
        return Some(vec![MicroOp::BlHigh(offset as u32)]);
    }
    if instr >> 11 == 0b11111 {
        let offset = ((instr & 0x7FF) as u32) << 1;
        return Some(vec![
            MicroOp::Internal,
            MicroOp::Internal,
            MicroOp::BlLow(offset),
        ]);
    }
    // BX (the whole 0x4700 range is op 0b11): interworking branch.
    if (instr & 0xFF00) == 0x4700 {
        let rs = ((instr >> 3) & 0xF) as usize;
        return Some(vec![
            MicroOp::Internal,
            MicroOp::Internal,
            MicroOp::Bx(regs.r(rs)),
        ]);
    }
    // SWI: trap to the BIOS HLE (or the SVC vector when unhandled).
    if (instr & 0xFF00) == 0xDF00 {
        return Some(vec![MicroOp::TrapSwi((instr & 0xFF) as u8)]);
    }
    // UND: the 0xDE00 range traps to the undefined vector.
    if (instr & 0xFF00) == 0xDE00 {
        return Some(vec![MicroOp::TrapUnd]);
    }
    // UND: decoder gaps (0xB100-0xB3FF, 0xB600-0xBBFF, 0xBE00-0xBFFF)
    // fall into the legacy decoder's `_` arm (`handle_undefined`).
    if (0xB100..=0xB3FF).contains(&instr)
        || (0xB600..=0xBBFF).contains(&instr)
        || (0xBE00..=0xBFFF).contains(&instr)
    {
        return Some(vec![MicroOp::TrapUnd]);
    }
    // MOV/CMP/ADD/SUB immediate.
    if instr >> 13 == 0b001 {
        let op = match (instr >> 11) & 0x3 {
            0b00 => AluImmOp::Mov,
            0b01 => AluImmOp::Cmp,
            0b10 => AluImmOp::Add,
            _ => AluImmOp::Sub,
        };
        let rd = ((instr >> 8) & 0x7) as usize;
        let imm = (instr & 0xFF) as u32;
        return Some(vec![MicroOp::CommitAlu(AluEffect {
            op,
            rd,
            rn: rd,
            imm,
            set_flags: true,
            thumb_mov: matches!(op, AluImmOp::Mov),
            carry: None,
        })]);
    }
    // Thumb MUL (op 0xD in 0x4000..=0x43FF): m from the incoming Rd,
    // mirroring the decoder preamble; other ALU ops stay legacy.
    if (instr & 0xFFC0) == 0x4340 {
        let m = multiplier_cycles(regs.r((instr & 0x7) as usize));
        let mut ops = vec![MicroOp::CommitMul(MulEffect {
            instr: u32::from(instr),
            thumb: true,
        })];
        ops.extend(vec![MicroOp::Internal; m as usize]);
        return Some(ops);
    }
    if let Some(ops) = expand_thumb_alu_rest(instr) {
        return Some(ops);
    }
    expand_thumb_push_pop(instr, regs)
        .or_else(|| expand_thumb_multiple(instr, regs))
        .or_else(|| expand_thumb_pcrel(instr, regs))
        .or_else(|| expand_thumb_load_store(instr))
}

/// Thumb LDR (literal) 0x4800..=0x4FFF. Expands to [Read, I, I] (= 3),
/// matching `handle_pc_relative` (bus read32, then fetch-stream-break).
fn expand_thumb_pcrel(instr: u16, regs: &CpuRegisters) -> Option<Vec<MicroOp>> {
    if !(0x4800..=0x4FFF).contains(&instr) {
        return None;
    }
    let addr = (regs.pc() & !3).wrapping_add(((instr & 0xFF) as u32) << 2);
    Some(vec![
        MicroOp::PcRelRead(PcRelRead {
            addr,
            rd: ((instr >> 8) & 0x7) as usize,
        }),
        MicroOp::Internal,
        MicroOp::Internal,
    ])
}

/// Thumb LDMIA/STMIA, including the empty forms (single PC word with
/// +0x40 SP arithmetic on LDM, single R15+2 store on STM). Same block
/// slicing as PUSH/POP: per-word continuation at execution, batch
/// open/close and the single fetch-stream break instruction-scoped,
/// base writeback (LDM skips it when the base is loaded) in the end
/// commit.
fn expand_thumb_multiple(instr: u16, regs: &CpuRegisters) -> Option<Vec<MicroOp>> {
    if instr >> 12 != 0xC {
        return None;
    }
    let load = (instr >> 11) & 1 == 1;
    let rb = ((instr >> 8) & 0x7) as usize;
    let rlist = instr & 0xFF;
    let base = regs.r(rb);
    if rlist == 0 {
        // Empty LDMIA/STMIA: the single PC word at [Rb] with Rb
        // advancing 0x40 (NOT the PUSH/POP shape: no batch framing,
        // writeback through `set_r`, store R15+2). Mirrors
        // `handle_empty_multiple` exactly (bases 5/2).
        let mut ops = vec![MicroOp::BlockEmpty(BlockEmptyEffect {
            load,
            addr: base,
            writeback_reg: Some((rb, base.wrapping_add(0x40))),
            store_value: regs.pc().wrapping_add(2),
            restore_cpsr: false,
            reset_sequential: false,
            // Standalone (no BlockEnd follows): break here, like legacy.
            break_stream: true,
        })];
        ops.extend(vec![MicroOp::Internal; if load { 4 } else { 1 }]);
        return Some(ops);
    }
    let count = rlist.count_ones();
    let slots: Vec<usize> = (0..8).filter(|i| rlist & (1 << i) != 0).collect();
    // STM stored-base quirk: a non-first occurrence of the base stores
    // the final address (legacy `stm_value`).
    let final_addr = base.wrapping_add(count * 4);
    let first = rlist.trailing_zeros() as usize;
    let mut ops = vec![MicroOp::BlockStart(BlockStartEffect {
        is_load: load,
        fetch_width: 2,
    })];
    for (i, reg) in slots.iter().enumerate() {
        let store_value = if load {
            None
        } else {
            Some(if *reg == rb && rb != first {
                final_addr
            } else {
                regs.r(*reg)
            })
        };
        ops.push(MicroOp::BlockWord(BlockWord {
            addr: base.wrapping_add(i as u32 * 4),
            reg: *reg,
            load,
            first: i == 0,
            pc_load: false,
            user_bank: false,
            restore_cpsr: false,
            store_value,
        }));
    }
    // LDM writes the base back only when it is not itself loaded.
    let writeback = if load && (rlist >> rb) & 1 != 0 {
        None
    } else {
        Some((rb, final_addr))
    };
    ops.push(MicroOp::BlockEnd(BlockEndEffect {
        sp: None,
        writeback,
        ldm_conflict: false,
        first_addr: base,
    }));
    // Pad the legacy handler base (LDM 2+count, STM 1+count), except
    // single-register Thumb LDM (Break's ldmia r2!,{r3}): HW retires it
    // like a single LDR (1I, not 2I). Multi-word blocks (Timing OAM
    // 5-word cells pin 2I) keep 2.
    ops.extend(vec![
        MicroOp::Internal;
        if load {
            if count == 1 { 1 } else { 2 }
        } else {
            1
        }
    ]);
    Some(ops)
}

/// Thumb ALU remainder: move-shifted (0x0000..=0x17FF, 1 cycle),
/// ADD/SUB (0x1800..=0x1FFF, 1 cycle), register ALU (0x4000..=0x43FF
/// except MUL: 1 cycle, 2 for register shifts), hi-reg
/// (0x4400..=0x47FF except BX: 1 cycle, 3 for ADD/MOV to PC),
/// ADD SP/PC (0xA000..=0xAFFF) and SP offset (0xB000..=0xB0FF).
/// Expansion is [CommitThumb] padded to the legacy base; the commit
/// delegates, so only the cycle split is new.
pub(crate) fn expand_thumb_alu_rest(instr: u16) -> Option<Vec<MicroOp>> {
    let trailing: usize = if instr <= 0x1FFF || (0xA000..=0xB0FF).contains(&instr) {
        0
    } else if (0x4000..=0x43FF).contains(&instr) && ((instr >> 6) & 0xF) != 0xD {
        let op = ((instr >> 6) & 0xF) as u8;
        if matches!(op, 0x2..=0x4 | 0x7) { 1 } else { 0 }
    } else if (0x4400..=0x47FF).contains(&instr) && (instr & 0xFF00) != 0x4700 {
        let op = (instr >> 8) & 0b11;
        let rd = (instr & 0x7) + if (instr >> 7) & 1 != 0 { 8 } else { 0 };
        if rd == 15 && (op == 0b00 || op == 0b10) {
            2
        } else {
            0
        }
    } else {
        return None;
    };
    let mut ops = vec![MicroOp::CommitThumb(instr)];
    ops.extend(vec![MicroOp::Internal; trailing]);
    Some(ops)
}

/// Thumb PUSH/POP, including the empty forms (single PC word with
/// +0x40 SP arithmetic on POP, single R15+2 store on PUSH). Mirrors
/// the decoder ranges exactly (subset of the legacy route, no extra
/// validation: the handler itself does not validate either).
fn expand_thumb_push_pop(instr: u16, regs: &CpuRegisters) -> Option<Vec<MicroOp>> {
    let push = match instr {
        0xB400..=0xB5FF => true,
        0xBC00..=0xBDFF => false,
        _ => return None,
    };
    let sp = regs.sp();
    let extra = instr & (1 << 8) != 0; // LR for PUSH, PC for POP
    let list = instr & 0xFF;
    let count = list.count_ones() + u32::from(extra);
    if count == 0 {
        return Some(expand_thumb_push_pop_empty(push, sp, regs));
    }
    let mut slots: Vec<(usize, bool)> = (0..8)
        .filter(|r| list & (1 << r) != 0)
        .map(|r| (r as usize, false))
        .collect();
    if extra {
        slots.push((if push { 14 } else { 15 }, !push));
    }
    let mut ops = vec![MicroOp::BlockStart(BlockStartEffect {
        is_load: !push,
        fetch_width: 2,
    })];
    for (i, (reg, pc_load)) in slots.iter().enumerate() {
        let addr = if push {
            sp.wrapping_sub(count * 4).wrapping_add(i as u32 * 4)
        } else {
            sp.wrapping_add(i as u32 * 4)
        };
        ops.push(MicroOp::BlockWord(BlockWord {
            addr,
            reg: *reg,
            load: !push,
            first: i == 0,
            pc_load: *pc_load,
            user_bank: false,
            restore_cpsr: false,
            store_value: None,
        }));
    }
    ops.push(MicroOp::BlockEnd(BlockEndEffect {
        sp: Some(if push {
            sp.wrapping_sub(count * 4)
        } else {
            sp.wrapping_add(count * 4)
        }),
        writeback: None,
        ldm_conflict: false,
        first_addr: if push { sp.wrapping_sub(count * 4) } else { sp },
    }));
    // Pad the legacy handler base: PUSH 1+count, POP 2+count,
    // POP+PC 4+count (words already carry +1 each).
    let trailing = if push {
        1
    } else if extra {
        4
    } else {
        2
    };
    ops.extend(vec![MicroOp::Internal; trailing as usize]);
    Some(ops)
}

/// Thumb empty PUSH/POP: PUSH stores R15+2 with SP moving one word
/// (base 2); POP loads PC with SP advancing 0x40 (base 6). Batched
/// like the legacy empty path; the op carries +1 with trailing
/// Internals to the base.
fn expand_thumb_push_pop_empty(push: bool, sp: u32, regs: &CpuRegisters) -> Vec<MicroOp> {
    let addr = if push { sp.wrapping_sub(4) } else { sp };
    let mut ops = vec![
        MicroOp::BlockStart(BlockStartEffect {
            is_load: !push,
            fetch_width: 2,
        }),
        MicroOp::BlockEmpty(BlockEmptyEffect {
            load: !push,
            addr,
            writeback_reg: None,
            store_value: regs.pc().wrapping_add(2),
            restore_cpsr: false,
            reset_sequential: true,
            // The trailing BlockEnd carries the single break.
            break_stream: false,
        }),
        MicroOp::BlockEnd(BlockEndEffect {
            sp: Some(if push {
                sp.wrapping_sub(4)
            } else {
                sp.wrapping_add(0x40)
            }),
            writeback: None,
            ldm_conflict: false,
            first_addr: addr,
        }),
    ];
    // Pad the legacy handler base: empty PUSH 2, empty POP 6
    // (the op already carries +1).
    ops.extend(vec![MicroOp::Internal; if push { 1 } else { 5 }]);
    ops
}

/// Shared immediate-form load/store shape: [Read, I, I] / [Write, I].
fn imm_access_ops(load: bool, acc: MemAccess) -> Vec<MicroOp> {
    if load {
        vec![MicroOp::MemRead(acc), MicroOp::Internal, MicroOp::Internal]
    } else {
        vec![MicroOp::MemWrite(acc), MicroOp::Internal]
    }
}

/// Thumb word LDR/STR (immediate offset and SP-relative) and
/// LDRH/STRH immediate. Same [Read, I, I] / [Write, I] shape as ARM.
pub fn expand_thumb_load_store(instr: u16) -> Option<Vec<MicroOp>> {
    // Register-offset word/byte and signed/halfword transfers (0101).
    if instr >> 12 == 0b0101 {
        let op = (instr >> 10) & 0x3;
        let ro = ((instr >> 6) & 0x7) as usize;
        let rb = ((instr >> 3) & 0x7) as usize;
        let rd = (instr & 0x7) as usize;
        let signed_group = instr & 0x0200 != 0;
        let (load, width, signed_load) = if signed_group {
            match op {
                0b00 => (false, 2, false), // STRH
                0b01 => (true, 1, true),   // LDRSB
                0b10 => (true, 2, false),  // LDRH
                _ => (true, 2, true),      // LDRSH
            }
        } else {
            (
                instr & (1 << 11) != 0,
                if instr & (1 << 10) != 0 { 1 } else { 4 },
                false,
            )
        };
        let acc = MemAccess {
            width,
            rd,
            rn: rb,
            offset: 0,
            offset_register: Some(ro),
            subtract: false,
            is_sp: false,
            signed_load,
            post_indexed: false,
            writeback: false,
            store_value: None,
            // Only the Thumb LDRSH carries the odd-address bus quirk.
            halfword_odd_quirk: load && width == 2 && signed_load,
        };
        return Some(imm_access_ops(load, acc));
    }
    // SP-relative (1001): addr = SP + imm8<<2.
    if instr >> 12 == 0b1001 {
        let l = (instr >> 11) & 1 == 1;
        let rd = ((instr >> 8) & 0x7) as usize;
        let acc = MemAccess {
            width: 4,
            rd,
            rn: 13,
            offset: ((instr & 0xFF) as u32) << 2,
            offset_register: None,
            subtract: false,
            is_sp: true,
            signed_load: false,
            post_indexed: false,
            writeback: false,
            store_value: None,
            halfword_odd_quirk: false,
        };
        // Totals match legacy handler returns (load 3, store 2).
        return Some(imm_access_ops(l, acc));
    }
    // Immediate-offset word (011, B == 0).
    if instr >> 13 == 0b011 && (instr >> 12) & 1 == 0 {
        let l = (instr >> 11) & 1 == 1;
        let rb = ((instr >> 3) & 0x7) as usize;
        let rd = (instr & 0x7) as usize;
        let acc = MemAccess {
            width: 4,
            rd,
            rn: rb,
            offset: (((instr >> 6) & 0x1F) as u32) << 2,
            offset_register: None,
            subtract: false,
            is_sp: false,
            signed_load: false,
            post_indexed: false,
            writeback: false,
            store_value: None,
            halfword_odd_quirk: false,
        };
        // Totals match legacy handler returns (load 3, store 2).
        return Some(imm_access_ops(l, acc));
    }
    // Immediate-offset byte (011, B == 1): offset is imm5 unshifted.
    if instr >> 13 == 0b011 && (instr >> 12) & 1 == 1 {
        let l = (instr >> 11) & 1 == 1;
        let rb = ((instr >> 3) & 0x7) as usize;
        let rd = (instr & 0x7) as usize;
        let acc = MemAccess {
            width: 1,
            rd,
            rn: rb,
            offset: ((instr >> 6) & 0x1F) as u32,
            offset_register: None,
            subtract: false,
            is_sp: false,
            signed_load: false,
            post_indexed: false,
            writeback: false,
            store_value: None,
            halfword_odd_quirk: false,
        };
        // Totals match legacy handler returns (load 3, store 2).
        return Some(imm_access_ops(l, acc));
    }
    // Halfword immediate (1000).
    if instr >> 12 == 0b1000 {
        let l = (instr >> 11) & 1 == 1;
        let rb = ((instr >> 3) & 0x7) as usize;
        let rd = (instr & 0x7) as usize;
        let acc = MemAccess {
            width: 2,
            rd,
            rn: rb,
            offset: (((instr >> 6) & 0x1F) as u32) << 1,
            offset_register: None,
            subtract: false,
            is_sp: false,
            signed_load: false,
            post_indexed: false,
            writeback: false,
            store_value: None,
            halfword_odd_quirk: false,
        };
        // Totals match legacy handler returns (load 3, store 2).
        return Some(imm_access_ops(l, acc));
    }
    None
}
