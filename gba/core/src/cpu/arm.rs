use crate::cpu::registers::CpuRegisters;
use crate::memory::GbaMemoryBus;

/// ARM命令デコーダ。condチェック後、カテゴリ別ハンドラへ振り分け。
pub fn decode_arm(regs: &mut CpuRegisters, bus: &mut GbaMemoryBus, instr: u32) -> u32 {
    let cond = ((instr >> 28) & 0xF) as u8;
    if cond != 0xE && !check_cond(regs.cpsr(), cond) {
        return 1; // 条件不一致 → 1S NOP
    }

    let bits27_25 = (instr >> 25) & 0b111;
    let _bit4 = (instr >> 4) & 1;
    let _bit7 = (instr >> 7) & 1;

    // Coprocessor / UND — GBAでは未定義
    if bits27_25 == 0b110 || bits27_25 == 0b111 && (instr >> 24) & 1 == 0 {
        // LDC/STC/CDP/MCR/MRC はUND例外 TODO(gba-coprocessor-und)
        return handle_und(regs);
    }

    match bits27_25 {
        0b000 | 0b001 => {
            // Data Processing / PSR Transfer / Multiply
            if (instr & 0x0FC000F0) == 0x00000090 {
                // Multiply
                return handle_multiply(regs, bus, instr);
            }
            if (instr & 0x0FBF0FFF) == 0x010F0000 {
                return handle_psr(regs, instr);
            }
            if (instr & 0x0FFFFFF0) == 0x012FFF10 {
                return handle_bx(regs, instr);
            }
            handle_data_processing(regs, bus, instr)
        }
        0b010 | 0b011 => handle_single_transfer(regs, bus, instr),
        0b100 => handle_block_transfer(regs, bus, instr),
        0b101 => handle_branch(regs, bus, instr),
        0b110 => handle_block_transfer(regs, bus, instr), // LDM/STM already
        0b111 => {
            if (instr >> 24) & 1 == 1 {
                handle_swi(regs, bus, instr)
            } else {
                handle_coprocessor_und(regs)
            }
        }
        _ => handle_und(regs),
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
    // TODO(gba-coprocessor-und): SPSR_und←CPSR, LR_und←PC+4, CPSR=T=0/M=UND, PC=0x04
    // Phase 5では空ハンドラとして1サイクルで NOP
    let _ = regs;
    1
}

fn handle_coprocessor_und(regs: &mut CpuRegisters) -> u32 {
    handle_und(regs)
}

fn handle_data_processing(_regs: &mut CpuRegisters, _bus: &mut GbaMemoryBus, _instr: u32) -> u32 {
    // TODO: barrel_shift + ALU
    1
}
fn handle_multiply(_regs: &mut CpuRegisters, _bus: &mut GbaMemoryBus, _instr: u32) -> u32 {
    1
}
fn handle_psr(_regs: &mut CpuRegisters, _instr: u32) -> u32 {
    1
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
fn handle_single_transfer(_regs: &mut CpuRegisters, _bus: &mut GbaMemoryBus, _instr: u32) -> u32 {
    2
}
fn handle_block_transfer(_regs: &mut CpuRegisters, _bus: &mut GbaMemoryBus, _instr: u32) -> u32 {
    2
}
fn handle_branch(_regs: &mut CpuRegisters, _bus: &mut GbaMemoryBus, _instr: u32) -> u32 {
    3
}
fn handle_swi(_regs: &mut CpuRegisters, _bus: &mut GbaMemoryBus, _instr: u32) -> u32 {
    2
}

#[cfg(test)]
mod tests {
    use crate::cpu::registers::CpuRegisters;

    use super::check_cond;

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
}
