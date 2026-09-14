use nerust_gba_core::cpu_registers::CpuRegisters;

#[test]
fn tmp_msr_irq_to_sys() {
    let mut regs = CpuRegisters::post_bios();
    regs.set_cpsr(0x1F); // SYS
    regs.set_r(14, 0x08010200); // USR LR (BL-intermediate)
    regs.set_cpsr(0x12); // IRQ (banks USR out)
    regs.set_r(14, 0x14); // HLE trampoline clobbers LR_irq
    regs.set_cpsr(0x1F); // handler msr SYS
    eprintln!("mode={:#x} r14={:#x}", regs.cpsr_mode(), regs.r(14));
    assert_eq!(regs.cpsr_mode(), 0x1F);
    assert_eq!(regs.r(14), 0x08010200);
}
