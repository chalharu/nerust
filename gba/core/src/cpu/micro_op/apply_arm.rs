//! ARM micro-op execution: SWP/PSR/DP-register commits and traps.
//! Leaf module: depends only on the op types, registers, bus, and
//! the pure [`semantics`](crate::cpu::semantics) helpers.
use crate::cpu::semantics::{barrel_shift, barrel_shift_register, update_nz};
use crate::cpu_registers::CpuRegisters;
use crate::memory::GbaMemoryBus;

/// SWP/SWPB (GBATEK ARM Single Data Swap): native micro-op
/// implementation mirroring `arm_opcodes::swp::handle` exactly
/// (atomic load-then-store with the HW bus lock: no DMA between the
/// pair, so expansion keeps it a single commit).
pub(super) fn apply_swp(regs: &mut CpuRegisters, bus: &mut GbaMemoryBus, instr: u32) -> u32 {
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
    // Rd == R15 is UNPREDICTABLE (ARM ARM): skip the write like MUL.
    if rd != 15 {
        regs.set_r(rd, mem_val);
    }
    // Load+store breaks the fetch stream (once per instruction).
    bus.charge_fetch_stream_break(addr);
    4
}

/// MRS/MSR: native micro-op implementation mirroring
/// `arm_opcodes::psr_transfer::handle` exactly.
pub(super) fn apply_psr(regs: &mut CpuRegisters, instr: u32) -> u32 {
    let psr = (instr >> 22) & 1 != 0; // 0=CPSR, 1=SPSR
    let is_mrs = (instr >> 21) & 1 == 0 && (instr & 0x0FBF0FFF) == 0x010F0000;
    if is_mrs {
        // MRS copies the selected status register into Rd.
        psr_read(regs, instr, psr);
    } else {
        psr_write(regs, instr, psr);
    }
    1
}

fn psr_read(regs: &mut CpuRegisters, instr: u32, saved: bool) {
    let rd = ((instr >> 12) & 0xF) as usize;
    // MRS with Rd=R15 is UNPREDICTABLE and must not branch: ignore the
    // write instead of latching PC to the PSR value via set_r(15).
    if rd == 15 {
        return;
    }
    let value = if saved { regs.spsr() } else { regs.cpsr() };
    regs.set_r(rd, value);
}

fn psr_write(regs: &mut CpuRegisters, instr: u32, saved: bool) {
    let operand = psr_operand(regs, instr);
    let mut field_mask = (instr >> 16) & 0xF;
    if regs.cpsr_mode() == 0x10 {
        field_mask &= 0x8;
    }
    let current = if saved { regs.spsr() } else { regs.cpsr() };
    let mut value = psr_apply_fields(current, operand, field_mask);
    if !saved {
        // GBATEK ARM PSR Transfer: "The T-bit may not be changed; for
        // THUMB/ARM switching use BX". Keep the live T bit on MSR to CPSR
        // (SPSR writes keep bit 5, which exception entry stores itself).
        value = (value & !(1 << 5)) | (current & (1 << 5));
        // Writing an illegal mode (< 0x10) keeps bit 4 set (HW-tested:
        // MSR 0x03 lands in 0x13, not 0x03).
        if value & 0x10 == 0 {
            value |= 0x10;
        }
    }
    if saved {
        regs.set_spsr(value);
    } else {
        regs.set_cpsr(value);
    }
}

fn psr_operand(regs: &CpuRegisters, instr: u32) -> u32 {
    if (instr >> 25) & 1 == 0 {
        return regs.r((instr & 0xF) as usize);
    }
    (instr & 0xFF).rotate_right(((instr >> 8) & 0xF) * 2)
}

fn psr_apply_fields(mut current: u32, operand: u32, mask: u32) -> u32 {
    // Field mask encoding: bit0=c, bit1=x, bit2=s, bit3=f.
    // ARM7TDMI defines control and NZCV fields here; reserved x/s bits stay unchanged.
    if mask & 1 != 0 {
        current = (current & 0xFFFFFF00) | (operand & 0xFF);
    }
    if mask & 8 != 0 {
        current = (current & 0x0FFFFFFF) | (operand & 0xF0000000);
    }
    current
}

/// ARM data-processing (register form, I==0): native micro-op
/// implementation mirroring `arm_opcodes::data_processing::handle`
/// exactly, including the register-shift +1I and R15-write refill.
pub(super) fn apply_dp_reg(regs: &mut CpuRegisters, instr: u32) -> u32 {
    let i = (instr >> 25) & 1 != 0;
    let opcode = ((instr >> 21) & 0xF) as u8;
    let s = (instr >> 20) & 1 != 0;
    let rn = ((instr >> 16) & 0xF) as usize;
    let rd = ((instr >> 12) & 0xF) as usize;
    let register_shift = !i && (instr >> 4) & 1 != 0;
    if register_shift {
        // Register-specified shifts take an internal cycle (GBATEK);
        // carried in the base below, no fetch-stream break.
    }
    // Operand2 always passes through the barrel shifter, including immediates.
    let (op2, shifter_carry) = dp_operand2(regs, instr, i);
    let rn_value = dp_register_operand(regs, rn, register_shift);
    let (result, carry, overflow) = dp_execute(opcode, rn_value, op2, shifter_carry, regs.cpsr_c());
    // TST/TEQ/CMP/CMN update flags without writing Rd.
    let flag_only = matches!(opcode, 0x8..=0xB);
    // USR/SYS have no SPSR (ARM ARM): exception-return restores only
    // restores only apply in modes with an SPSR bank.
    let has_spsr = !matches!(regs.cpsr_mode(), 0x10 | 0x1F);
    if flag_only && rd == 15 && s {
        // Unpredictable on later ARM cores; ARM7TDMI restores CPSR without writing PC.
        if has_spsr {
            regs.set_cpsr(regs.spsr());
            // Exception return: pipeline refill (+1S+1N).
            return 3 + u32::from(register_shift);
        }
        dp_update_flags(regs, opcode, result, carry, overflow);
        return 1 + u32::from(register_shift);
    }
    if !flag_only {
        dp_write_result(regs, rd, result, s);
    }
    if s && (!(rd == 15 && !flag_only) || !has_spsr) {
        dp_update_flags(regs, opcode, result, carry, overflow);
    }
    // GBATEK: data processing = 1S (+1I if SHIFT(Rs), +1S+1N if R15
    // written — pipeline refill, same convention as B).
    let mut cycles = 1 + u32::from(register_shift);
    if rd == 15 && !flag_only {
        cycles += 2;
    }
    cycles
}

fn dp_operand2(regs: &CpuRegisters, instr: u32, immediate: bool) -> (u32, bool) {
    if immediate {
        let imm = instr & 0xFF;
        let rot = ((instr >> 8) & 0xF) * 2;
        return dp_immediate_operand(imm, rot, regs.cpsr_c());
    }
    let rm = (instr & 0xF) as usize;
    let shift_type = ((instr >> 5) & 0b11) as u8;
    if (instr >> 4) & 1 != 0 {
        let value = dp_register_operand(regs, rm, true);
        let amount = regs.r(((instr >> 8) & 0xF) as usize) & 0xFF;
        barrel_shift_register(value, shift_type, amount, regs.cpsr_c())
    } else {
        let value = regs.r(rm);
        barrel_shift(value, shift_type, (instr >> 7) & 0x1F, regs.cpsr_c())
    }
}

fn dp_register_operand(regs: &CpuRegisters, register: usize, register_shift: bool) -> u32 {
    regs.r(register)
        .wrapping_add(u32::from(register == 15 && register_shift) * 4)
}

fn dp_immediate_operand(value: u32, rotation: u32, carry: bool) -> (u32, bool) {
    if rotation == 0 {
        (value, carry)
    } else {
        (
            value.rotate_right(rotation),
            value & (1 << (rotation - 1)) != 0,
        )
    }
}

fn dp_execute(
    opcode: u8,
    left: u32,
    right: u32,
    shift_carry: bool,
    carry: bool,
) -> (u32, bool, bool) {
    // Opcode map: AND, EOR, SUB, RSB, ADD, ADC, SBC, RSC,
    // TST, TEQ, CMP, CMN, ORR, MOV, BIC, MVN.
    match opcode {
        0x0 => (left & right, shift_carry, false),   // AND
        0x1 => (left ^ right, shift_carry, false),   // EOR
        0x2 | 0xA => dp_sub_with_flags(left, right), // SUB/CMP
        0x3 => dp_sub_with_flags(right, left),       // RSB
        0x4 | 0xB => dp_add_with_flags(left, right), // ADD/CMN
        0x5 => dp_adc_with_flags(left, right, u32::from(carry)), // ADC
        0x6 => dp_sbc_with_flags(left, right, u32::from(carry)), // SBC
        0x7 => dp_sbc_with_flags(right, left, u32::from(carry)), // RSC
        0x8 => (left & right, shift_carry, false),   // TST
        0x9 => (left ^ right, shift_carry, false),   // TEQ
        0xC => (left | right, shift_carry, false),   // ORR
        0xD => (right, shift_carry, false),          // MOV
        0xE => (left & !right, shift_carry, false),  // BIC
        0xF => (!right, shift_carry, false),         // MVN
        _ => unreachable!(),
    }
}

fn dp_write_result(regs: &mut CpuRegisters, destination: usize, result: u32, set_flags: bool) {
    regs.set_r(destination, result);
    if destination == 15 && set_flags {
        // Data-processing with S and Rd=PC returns from an exception via
        // SPSR — except in USR/SYS, which have none (ARM ARM): there
        // the PC write stands and flags update at the call site.
        if !matches!(regs.cpsr_mode(), 0x10 | 0x1F) {
            regs.set_cpsr(regs.spsr());
        }
    }
}

fn dp_update_flags(regs: &mut CpuRegisters, opcode: u8, result: u32, carry: bool, overflow: bool) {
    update_nz(regs, result);
    regs.set_cpsr_c(carry);
    // Logical operations preserve V; arithmetic operations replace it.
    if !matches!(opcode, 0x0 | 0x1 | 0x8 | 0x9 | 0xC..=0xF) {
        regs.set_cpsr_v(overflow);
    }
}

fn dp_add_with_flags(a: u32, b: u32) -> (u32, bool, bool) {
    let (r, c) = a.overflowing_add(b);
    let v = ((a ^ r) & (b ^ r) & 0x8000_0000) != 0;
    (r, c, v)
}

fn dp_sub_with_flags(a: u32, b: u32) -> (u32, bool, bool) {
    let (r, c) = a.overflowing_sub(b);
    let v = ((a ^ b) & (a ^ r) & 0x8000_0000) != 0;
    // Carry is NOT borrow
    (r, !c, v) // overflowing_sub returns borrow as carry; invert
}

fn dp_adc_with_flags(a: u32, b: u32, c_in: u32) -> (u32, bool, bool) {
    let (r1, c1) = a.overflowing_add(b);
    let (r, c2) = r1.overflowing_add(c_in);
    let c = c1 || c2;
    let signed = a as i32 as i64 + b as i32 as i64 + i64::from(c_in);
    let v = signed > i64::from(i32::MAX) || signed < i64::from(i32::MIN);
    (r, c, v)
}

fn dp_sbc_with_flags(a: u32, b: u32, c_in: u32) -> (u32, bool, bool) {
    // SBC = A - B - !C
    let not_c = 1 - c_in;
    let (r1, c1) = a.overflowing_sub(b);
    let (r, c2) = r1.overflowing_sub(not_c);
    let c = !(c1 || c2);
    let signed = a as i32 as i64 - b as i32 as i64 - i64::from(not_c);
    let v = signed > i64::from(i32::MAX) || signed < i64::from(i32::MIN);
    (r, c, v)
}

/// SWI trap: run the BIOS HLE dispatcher and carry its full charge
/// (SVC-vector entry on Unsupported), mirroring the legacy SWI
/// handlers (`arm_opcodes::swi`, `thumb_opcodes::branch`) exactly.
pub(super) fn apply_trap_swi(
    regs: &mut CpuRegisters,
    bus: &mut GbaMemoryBus,
    swi: u8,
    is_thumb: bool,
) -> u32 {
    let back = if is_thumb { 2 } else { 4 };
    match crate::bios::handle_swi(regs, bus, swi) {
        crate::bios::SwiResult::Return(cycles) => {
            regs.set_pc(regs.pc().wrapping_sub(back));
            cycles
        }
        crate::bios::SwiResult::Branch(cycles) => cycles,
        crate::bios::SwiResult::Unsupported => {
            let return_address = regs.pc().wrapping_sub(back);
            regs.enter_exception(0x13, 0x08, return_address, true);
            3
        }
    }
}

/// Undefined-instruction trap: exception entry, mirroring the legacy
/// UND handlers exactly.
pub(super) fn apply_trap_und(regs: &mut CpuRegisters, is_thumb: bool) -> u32 {
    let return_address = regs.pc().wrapping_sub(if is_thumb { 2 } else { 4 });
    regs.enter_exception(0x1B, 0x04, return_address, true);
    // GBATEK: Undefined = 2S+1I+1N = 4 in both states.
    4
}
