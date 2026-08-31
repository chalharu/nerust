use crate::cpu::registers::CpuRegisters;
use crate::memory::GbaMemoryBus;

use crate::cpu::thumb_opcodes;

/// Thumbデコーダ。Format 1-19 で振り分け。
pub fn decode_thumb(regs: &mut CpuRegisters, bus: &mut GbaMemoryBus, instr: u16) -> u32 {
    match instr {
        0x0000..=0x17FF => thumb_opcodes::move_shifted::handle(regs, instr),
        0x1800..=0x1FFF => thumb_opcodes::add_sub::handle(regs, instr),
        0x2000..=0x3FFF => thumb_opcodes::alu::handle_imm(regs, instr),
        0x4000..=0x43FF => thumb_opcodes::alu::handle(regs, instr),
        0x4400..=0x47FF => thumb_opcodes::hi_register::handle(regs, bus, instr),
        0x4800..=0x4FFF => thumb_opcodes::load_store::handle_pc_relative(regs, bus, instr),
        0x5000..=0x51FF | 0x5400..=0x55FF | 0x5800..=0x59FF | 0x5C00..=0x5DFF => {
            thumb_opcodes::load_store::handle_reg_offset(regs, bus, instr)
        }
        0x5200..=0x53FF | 0x5600..=0x57FF | 0x5A00..=0x5BFF | 0x5E00..=0x5FFF => {
            thumb_opcodes::load_store::handle_sign_extended(regs, bus, instr)
        }
        0x6000..=0x7FFF => thumb_opcodes::load_store::handle_imm_offset(regs, bus, instr),
        0x8000..=0x8FFF => thumb_opcodes::load_store::handle_halfword(regs, bus, instr),
        0x9000..=0x9FFF => thumb_opcodes::load_store::handle_sp_relative(regs, bus, instr),
        0xA000..=0xAFFF => thumb_opcodes::alu::handle_load_address(regs, instr),
        0xB000..=0xB0FF => thumb_opcodes::alu::handle_sp_offset(regs, instr),
        0xB400..=0xB5FF | 0xBC00..=0xBDFF => thumb_opcodes::push_pop::handle(regs, bus, instr),
        0xC000..=0xCFFF => thumb_opcodes::load_store::handle_multiple(regs, bus, instr),
        0xD000..=0xDDFF => thumb_opcodes::branch::handle_cond(regs, instr),
        0xDE00..=0xDEFF => thumb_opcodes::branch::handle_undefined(regs),
        0xDF00..=0xDFFF => thumb_opcodes::branch::handle_swi(regs, bus, instr),
        0xE000..=0xE7FF => thumb_opcodes::branch::handle_uncond(regs, instr),
        0xF000..=0xFFFF => thumb_opcodes::branch::handle_long_bl(regs, instr),
        _ => thumb_opcodes::branch::handle_undefined(regs),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn alu_format_reaches_alu_handler() {
        let mut regs = CpuRegisters::post_bios();
        regs.set_r(0, 1);
        regs.set_r(1, 2);
        let mut bus = GbaMemoryBus::new();
        decode_thumb(&mut regs, &mut bus, 0x4308); // ORR R0,R1
        assert_eq!(regs.r(0), 3);
    }

    #[test]
    fn pop_and_ldmia_are_reachable() {
        let mut regs = CpuRegisters::post_bios();
        let mut bus = GbaMemoryBus::new();
        regs.set_sp(0x03000000);
        bus.write32(0x03000000, 0x12345678);
        decode_thumb(&mut regs, &mut bus, 0xBC01); // POP {R0}
        assert_eq!(regs.r(0), 0x12345678);

        regs.set_r(1, 0x03000004);
        bus.write32(0x03000004, 0xCAFEBABE);
        decode_thumb(&mut regs, &mut bus, 0xC904); // LDMIA R1!, {R2}
        assert_eq!(regs.r(2), 0xCAFEBABE);
    }
}
