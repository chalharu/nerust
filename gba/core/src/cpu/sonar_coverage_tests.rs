use crate::{cpu_registers::CpuRegisters, memory::GbaMemoryBus};

#[test]
fn covers_thumb_alu_instruction_families() {
    let mut bus = GbaMemoryBus::new();
    for opcode in 0u16..16 {
        let mut regs = CpuRegisters::post_bios();
        regs.set_r(0, 0x8000_0001);
        regs.set_r(
            1,
            match opcode {
                2..=4 | 7 => 32,
                _ => 2,
            },
        );
        crate::cpu::thumb_opcodes::alu::handle(&mut regs, (opcode << 6) | (1 << 3));
    }
    for opcode in 0u16..4 {
        let mut regs = CpuRegisters::post_bios();
        regs.set_r(0, 1);
        crate::cpu::thumb_opcodes::alu::handle_imm(&mut regs, (opcode << 11) | 1);
    }
    let mut regs = CpuRegisters::post_bios();
    regs.set_sp(0x0300_0000);
    crate::cpu::thumb_opcodes::alu::handle_load_address(&mut regs, 0xA001);
    crate::cpu::thumb_opcodes::alu::handle_load_address(&mut regs, 0xA801);
    crate::cpu::thumb_opcodes::alu::handle_sp_offset(&mut regs, 1);
    crate::cpu::thumb_opcodes::alu::handle_sp_offset(&mut regs, 0x81);
    let _ = &mut bus;
}

#[test]
fn covers_thumb_shift_add_and_high_register_families() {
    for opcode in 0u16..3 {
        for amount in [0u16, 1] {
            let mut regs = CpuRegisters::post_bios();
            regs.set_r(1, 0x8000_0001);
            crate::cpu::thumb_opcodes::move_shifted::handle(
                &mut regs,
                (opcode << 11) | (amount << 6) | (1 << 3),
            );
        }
    }
    for immediate in [false, true] {
        for subtract in [false, true] {
            let mut regs = CpuRegisters::post_bios();
            regs.set_r(1, 4);
            regs.set_r(2, 2);
            let instruction = 0x1800
                | (u16::from(immediate) << 10)
                | (u16::from(subtract) << 9)
                | (2 << 6)
                | (1 << 3);
            crate::cpu::thumb_opcodes::add_sub::handle(&mut regs, instruction);
        }
    }
    let mut bus = GbaMemoryBus::new();
    for opcode in 0u16..4 {
        let mut regs = CpuRegisters::post_bios();
        regs.set_r(0, 1);
        regs.set_r(8, 0x0300_0001);
        let instruction = 0x4400 | (opcode << 8) | (1 << 7);
        crate::cpu::thumb_opcodes::hi_register::handle(&mut regs, &mut bus, instruction);
    }
}

#[test]
fn covers_thumb_load_store_families() {
    let mut regs = CpuRegisters::post_bios();
    let mut bus = GbaMemoryBus::new();
    regs.set_r(0, 0x1122_3344);
    regs.set_r(1, 0x0300_0000);
    regs.set_r(2, 4);
    regs.set_sp(0x0300_0100);

    for byte in [false, true] {
        for load in [false, true] {
            let instruction =
                (u16::from(load) << 11) | (u16::from(byte) << 10) | (2 << 6) | (1 << 3);
            crate::cpu::thumb_opcodes::load_store::handle_reg_offset(
                &mut regs,
                &mut bus,
                instruction,
            );
        }
    }
    for opcode in 0u16..4 {
        let instruction = 0x5200 | (opcode << 10) | (2 << 6) | (1 << 3);
        crate::cpu::thumb_opcodes::load_store::handle_sign_extended(
            &mut regs,
            &mut bus,
            instruction,
        );
    }
    for byte in [false, true] {
        for load in [false, true] {
            let instruction = 0x6000 | (u16::from(byte) << 12) | (u16::from(load) << 11) | (1 << 3);
            crate::cpu::thumb_opcodes::load_store::handle_imm_offset(
                &mut regs,
                &mut bus,
                instruction,
            );
        }
    }
    crate::cpu::thumb_opcodes::load_store::handle_halfword(&mut regs, &mut bus, 1 << 3);
    crate::cpu::thumb_opcodes::load_store::handle_halfword(
        &mut regs,
        &mut bus,
        (1 << 11) | (1 << 3),
    );
    crate::cpu::thumb_opcodes::load_store::handle_sp_relative(&mut regs, &mut bus, 0);
    crate::cpu::thumb_opcodes::load_store::handle_sp_relative(&mut regs, &mut bus, 1 << 11);
    crate::cpu::thumb_opcodes::load_store::handle_multiple(&mut regs, &mut bus, (1 << 8) | 1);
    regs.set_r(1, 0x0300_0000);
    crate::cpu::thumb_opcodes::load_store::handle_multiple(
        &mut regs,
        &mut bus,
        (1 << 11) | (1 << 8) | 1,
    );
    crate::cpu::thumb_opcodes::load_store::handle_pc_relative(&mut regs, &mut bus, 0);
}

#[test]
fn covers_thumb_stack_and_branch_families() {
    let mut regs = CpuRegisters::post_bios();
    let mut bus = GbaMemoryBus::new();
    regs.set_sp(0x0300_0100);
    regs.set_lr(0x0800_0001);
    regs.set_r(0, 0x1234);
    crate::cpu::thumb_opcodes::push_pop::handle(&mut regs, &mut bus, 0xB501);
    crate::cpu::thumb_opcodes::push_pop::handle(&mut regs, &mut bus, 0xBD01);

    regs.set_pc(0x0800_0004);
    regs.set_cpsr_z(true);
    crate::cpu::thumb_opcodes::branch::handle_cond(&mut regs, 0xD000);
    regs.set_cpsr_z(false);
    crate::cpu::thumb_opcodes::branch::handle_cond(&mut regs, 0xD000);
    crate::cpu::thumb_opcodes::branch::handle_uncond(&mut regs, 0xE000);
    crate::cpu::thumb_opcodes::branch::handle_long_bl(&mut regs, 0xF000);
    crate::cpu::thumb_opcodes::branch::handle_long_bl(&mut regs, 0xF800);
    crate::cpu::thumb_opcodes::branch::handle_swi(&mut regs, &mut bus, 0xDF0D);
    crate::cpu::thumb_opcodes::branch::handle_undefined(&mut regs);
}

#[test]
fn covers_arm_data_processing_opcodes() {
    let mut bus = GbaMemoryBus::new();
    for opcode in 0u32..16 {
        let mut regs = CpuRegisters::post_bios();
        regs.set_r(0, 0x7FFF_FFFF);
        regs.set_cpsr_c(true);
        let instruction = 0xE000_0000 | (1 << 25) | (opcode << 21) | (1 << 20) | 1;
        crate::cpu::arm_opcodes::data_processing::handle(&mut regs, &mut bus, instruction);
    }
    let mut regs = CpuRegisters::post_bios();
    regs.set_r(0, 1);
    regs.set_r(1, 1);
    regs.set_r(2, 1);
    crate::cpu::arm_opcodes::data_processing::handle(&mut regs, &mut bus, 0xE000_0210);
}
