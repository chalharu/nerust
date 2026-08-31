use crate::cpu_registers::CpuRegisters;
use crate::memory::GbaMemoryBus;

/// ARM命令デコーダ。condチェック後、カテゴリ別ハンドラへ振り分け。
pub fn decode_arm(regs: &mut CpuRegisters, bus: &mut GbaMemoryBus, instr: u32) -> u32 {
    let cond = ((instr >> 28) & 0xF) as u8;
    if cond != 0xE && !check_cond(regs.cpsr(), cond) {
        return 1; // 条件不一致 → 1S NOP
    }

    // Primary ARM classes: data/misc, single transfer, block transfer, branch,
    // coprocessor (undefined on GBA), and software interrupt.
    match (instr >> 25) & 0b111 {
        0b000 | 0b001 => decode_data_class(regs, bus, instr),
        0b010 | 0b011 => handle_single_transfer(regs, bus, instr),
        0b100 => handle_block_transfer(regs, bus, instr),
        0b101 => handle_branch(regs, bus, instr),
        0b110 => handle_und(regs),
        0b111 => decode_software_or_coprocessor(regs, bus, instr),
        _ => handle_und(regs),
    }
}

fn decode_data_class(regs: &mut CpuRegisters, bus: &mut GbaMemoryBus, instr: u32) -> u32 {
    // The 000/001 class overlaps data processing, multiply, SWP, PSR, BX,
    // and signed/halfword transfers. More-specific masks must be tested first.
    if (instr & 0x0F8000F0) == 0x00800090 || (instr & 0x0FC000F0) == 0x00000090 {
        return handle_multiply(regs, bus, instr);
    }
    if (instr & 0x0FB00FF0) == 0x01000090 {
        return handle_swp(regs, bus, instr);
    }
    if is_psr_transfer(instr) {
        return handle_psr(regs, instr);
    }
    if (instr & 0x0FFFFFF0) == 0x012FFF10 {
        return handle_bx(regs, instr);
    }
    if (instr & 0x00000090) == 0x00000090 {
        return handle_halfword(regs, bus, instr);
    }
    handle_data_processing(regs, bus, instr)
}

fn is_psr_transfer(instr: u32) -> bool {
    (instr & 0x0FBF0FFF) == 0x010F0000
        || (instr & 0x0FB0FFF0) == 0x0120F000
        || (instr & 0x0FB0F000) == 0x0320F000
}

fn decode_software_or_coprocessor(
    regs: &mut CpuRegisters,
    bus: &mut GbaMemoryBus,
    instr: u32,
) -> u32 {
    // ARM7TDMI in the GBA has no coprocessor; those encodings enter UND.
    if (instr >> 24) & 1 == 1 {
        handle_swi(regs, bus, instr)
    } else {
        handle_coprocessor_und(regs)
    }
}

fn check_cond(cpsr: u32, cond: u8) -> bool {
    let n = cpsr & (1 << 31) != 0;
    let z = cpsr & (1 << 30) != 0;
    let c = cpsr & (1 << 29) != 0;
    let v = cpsr & (1 << 28) != 0;
    match cond {
        0x0 => z,            // EQ
        0x1 => !z,           // NE
        0x2 => c,            // CS
        0x3 => !c,           // CC
        0x4 => n,            // MI
        0x5 => !n,           // PL
        0x6 => v,            // VS
        0x7 => !v,           // VC
        0x8 => c && !z,      // HI
        0x9 => !c || z,      // LS
        0xA => n == v,       // GE
        0xB => n != v,       // LT
        0xC => !z && n == v, // GT
        0xD => z || n != v,  // LE
        0xE => true,         // AL
        _ => false,          // NV
    }
}

fn handle_und(regs: &mut CpuRegisters) -> u32 {
    let return_address = regs.pc().wrapping_sub(4);
    regs.enter_exception(0x1B, 0x04, return_address, false);
    3
}

fn handle_coprocessor_und(regs: &mut CpuRegisters) -> u32 {
    handle_und(regs)
}

fn handle_data_processing(regs: &mut CpuRegisters, bus: &mut GbaMemoryBus, instr: u32) -> u32 {
    crate::cpu::arm_opcodes::data_processing::handle(regs, bus, instr)
}
fn handle_halfword(regs: &mut CpuRegisters, bus: &mut GbaMemoryBus, instr: u32) -> u32 {
    crate::cpu::arm_opcodes::halfword_transfer::handle(regs, bus, instr)
}
fn handle_swp(regs: &mut CpuRegisters, bus: &mut GbaMemoryBus, instr: u32) -> u32 {
    let b = (instr >> 22) & 1 != 0;
    let rn = ((instr >> 16) & 0xF) as usize;
    let rd = ((instr >> 12) & 0xF) as usize;
    let rm = (instr & 0xF) as usize;
    let addr = regs.r(rn);
    let rm_val = regs.r(rm);
    let mem_val = if b {
        bus.read8(addr) as u32
    } else {
        bus.read32(addr)
    };
    if b {
        bus.write8(addr, (rm_val & 0xFF) as u8);
    } else {
        bus.write32(addr, rm_val);
    }
    regs.set_r(rd, mem_val);
    4
}
fn handle_multiply(regs: &mut CpuRegisters, bus: &mut GbaMemoryBus, instr: u32) -> u32 {
    crate::cpu::arm_opcodes::multiply::handle(regs, bus, instr)
}
fn handle_psr(regs: &mut CpuRegisters, instr: u32) -> u32 {
    crate::cpu::arm_opcodes::psr_transfer::handle(regs, instr)
}
fn handle_bx(regs: &mut CpuRegisters, instr: u32) -> u32 {
    let rm = (instr & 0xF) as usize;
    let target = regs.r(rm);
    let thumb = target & 1 != 0;
    regs.set_cpsr((regs.cpsr() & !(1 << 5)) | ((thumb as u32) << 5));
    // pipeline flushは上位で処理
    regs.set_pc(target & !1);
    3
}
fn handle_single_transfer(regs: &mut CpuRegisters, bus: &mut GbaMemoryBus, instr: u32) -> u32 {
    crate::cpu::arm_opcodes::single_transfer::handle(regs, bus, instr)
}
fn handle_block_transfer(regs: &mut CpuRegisters, bus: &mut GbaMemoryBus, instr: u32) -> u32 {
    crate::cpu::arm_opcodes::block_transfer::handle(regs, bus, instr)
}
fn handle_branch(regs: &mut CpuRegisters, bus: &mut GbaMemoryBus, instr: u32) -> u32 {
    crate::cpu::arm_opcodes::branch::handle(regs, bus, instr)
}
fn handle_swi(regs: &mut CpuRegisters, bus: &mut GbaMemoryBus, instr: u32) -> u32 {
    crate::cpu::arm_opcodes::swi::handle(regs, bus, instr)
}

#[cfg(test)]
mod tests {
    use crate::cpu_registers::CpuRegisters;
    use crate::memory::GbaMemoryBus;

    use super::{check_cond, decode_arm};

    #[test]
    fn cond_eq() {
        let cpsr_z = 1 << 30;
        assert!(check_cond(cpsr_z, 0x0));
        assert!(!check_cond(0, 0x0));
    }

    #[test]
    fn cond_always() {
        assert!(check_cond(0, 0xE));
    }

    #[test]
    fn multiply_long_reaches_multiply_handler() {
        let mut regs = CpuRegisters::post_bios();
        let mut bus = GbaMemoryBus::new();
        regs.set_r(0, 3);
        regs.set_r(1, 4);
        // UMULL R2,R3,R0,R1
        decode_arm(&mut regs, &mut bus, 0xE0832190);
        assert_eq!(regs.r(2), 12);
        assert_eq!(regs.r(3), 0);
    }

    #[test]
    fn coprocessor_enters_undefined_exception() {
        let mut regs = CpuRegisters::post_bios();
        regs.set_pc(0x08000008);
        let old_cpsr = regs.cpsr();
        let mut bus = GbaMemoryBus::new();
        decode_arm(&mut regs, &mut bus, 0xEE000010);
        assert_eq!(regs.cpsr_mode(), 0x1B);
        assert_eq!(regs.spsr(), old_cpsr);
        assert_eq!(regs.lr(), 0x08000004);
        assert_eq!(regs.pc(), 0x04);
    }
}
