use crate::cpu::registers::CpuRegisters;
use crate::memory::GbaMemoryBus;

use crate::cpu::thumb_opcodes;

/// Thumbデコーダ。Format 1-19 で振り分け。
pub fn decode_thumb(regs: &mut CpuRegisters, bus: &mut GbaMemoryBus, instr: u16) -> u32 {
    let op = instr >> 11;
    match op {
        0b00000..=0b00010 => thumb_opcodes::move_shifted::handle(regs, instr),
        0b00011 => thumb_opcodes::add_sub::handle(regs, instr),
        0b00100..=0b00111 => thumb_opcodes::alu::handle_imm(regs, instr),
        0b01000 => {
            if (instr >> 6) & 0x3F == 0 {
                thumb_opcodes::alu::handle(regs, instr)
            } else {
                thumb_opcodes::hi_register::handle(regs, bus, instr)
            }
        }
        0b01001 => thumb_opcodes::load_store::handle_pc_relative(regs, bus, instr),
        0b01010..=0b01011 => thumb_opcodes::load_store::handle_reg_offset(regs, bus, instr),
        0b01100..=0b01111 => thumb_opcodes::load_store::handle_imm_offset(regs, bus, instr),
        0b10000 => thumb_opcodes::load_store::handle_halfword(regs, bus, instr),
        0b10010..=0b10011 => thumb_opcodes::load_store::handle_sp_relative(regs, bus, instr),
        0b10100..=0b10101 => thumb_opcodes::alu::handle_load_address(regs, instr),
        0b10110 => {
            if (instr >> 8) & 1 == 0 {
                thumb_opcodes::alu::handle_sp_offset(regs, instr)
            } else {
                thumb_opcodes::push_pop::handle(regs, bus, instr)
            }
        }
        0b11000 => thumb_opcodes::load_store::handle_multiple(regs, bus, instr),
        0b11010 => thumb_opcodes::branch::handle_cond(regs, instr),
        0b11011 => {
            if (instr >> 8) & 0xF == 0xF {
                thumb_opcodes::branch::handle_swi(regs, bus, instr)
            } else {
                thumb_opcodes::branch::handle_cond(regs, instr)
            }
        }
        0b11100 => thumb_opcodes::branch::handle_uncond(regs, instr),
        0b11110..=0b11111 => thumb_opcodes::branch::handle_long_bl(regs, instr),
        _ => 1,
    }
}
