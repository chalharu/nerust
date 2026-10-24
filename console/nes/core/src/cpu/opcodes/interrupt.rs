use super::{
    super::{
        Apu, ControllerHub, Core, CpuCartridgeBus, CpuStepState, CpuStepStateEnum, IRQ_VECTOR,
        IrqSource, NMI_VECTOR, Ppu, RESET_VECTOR, RegisterP, pull, push, read_dummy_current,
    },
    exit_opcode,
};

pub(crate) struct Brk;

impl CpuStepState for Brk {
    fn exec(
        core: &mut Core,
        ppu: &mut Ppu,
        cartridge: &mut dyn CpuCartridgeBus,
        hub: &mut dyn ControllerHub,
        apu: &mut Apu,
    ) -> CpuStepStateEnum {
        match core.internal_stat.get_step() {
            1 => {
                // dummy read
                let _ = core.memory.read_next(
                    &mut core.register,
                    ppu,
                    cartridge,
                    hub,
                    apu,
                    &mut core.interrupt,
                );
            }
            2 => {
                let pc = core.register.get_pc();
                let hi = (pc >> 8) as u8;
                core.internal_stat.set_data((pc & 0xFF) as u8);

                push(core, ppu, cartridge, hub, apu, hi);
            }
            3 => {
                push(
                    core,
                    ppu,
                    cartridge,
                    hub,
                    apu,
                    core.internal_stat.get_data(),
                );

                core.internal_stat.set_address(if core.interrupt.nmi {
                    // core.interrupt.nmi = false;
                    NMI_VECTOR
                } else {
                    IRQ_VECTOR
                });
            }
            4 => {
                let p = core.register.get_p() | (RegisterP::BREAK | RegisterP::RESERVED).bits();
                push(core, ppu, cartridge, hub, apu, p);
            }
            5 => {
                core.register.set_i(true);
                core.internal_stat.set_data(core.memory.read(
                    core.internal_stat.get_address(),
                    ppu,
                    cartridge,
                    hub,
                    apu,
                    &mut core.interrupt,
                ));
            }
            6 => {
                let hi = u16::from(core.memory.read(
                    core.internal_stat.get_address() + 1,
                    ppu,
                    cartridge,
                    hub,
                    apu,
                    &mut core.interrupt,
                ));
                core.register
                    .set_pc((hi << 8) | u16::from(core.internal_stat.get_data()));
            }
            _ => {
                core.interrupt.executing = false;
                return exit_opcode(core);
            }
        }
        CpuStepStateEnum::Continue
    }
}

pub(crate) struct Rti;

impl CpuStepState for Rti {
    fn exec(
        core: &mut Core,
        ppu: &mut Ppu,
        cartridge: &mut dyn CpuCartridgeBus,
        hub: &mut dyn ControllerHub,
        apu: &mut Apu,
    ) -> CpuStepStateEnum {
        match core.internal_stat.get_step() {
            1 => {
                // dummy read
                read_dummy_current(core, ppu, cartridge, hub, apu);
            }
            2 => {
                // dummy read
                let sp = usize::from(core.register.get_sp());
                let _ = core
                    .memory
                    .read(sp | 0x100, ppu, cartridge, hub, apu, &mut core.interrupt);
            }
            3 => {
                let p = pull(core, ppu, cartridge, hub, apu);
                core.register
                    .set_p((p & !(RegisterP::BREAK.bits())) | RegisterP::RESERVED.bits());
            }
            4 => {
                let data = pull(core, ppu, cartridge, hub, apu);
                core.internal_stat.set_data(data);
            }
            5 => {
                let high = pull(core, ppu, cartridge, hub, apu);
                core.register
                    .set_pc(u16::from(core.internal_stat.get_data()) | (u16::from(high) << 8));
            }
            _ => {
                return exit_opcode(core);
            }
        }
        CpuStepStateEnum::Continue
    }
}

pub(crate) struct Irq;

impl CpuStepState for Irq {
    fn exec(
        core: &mut Core,
        ppu: &mut Ppu,
        cartridge: &mut dyn CpuCartridgeBus,
        hub: &mut dyn ControllerHub,
        apu: &mut Apu,
    ) -> CpuStepStateEnum {
        match core.internal_stat.get_step() {
            1 => {
                // dummy read
                read_dummy_current(core, ppu, cartridge, hub, apu);
            }
            2 => {
                // dummy read
                read_dummy_current(core, ppu, cartridge, hub, apu);
            }
            3 => {
                let pc = core.register.get_pc();
                let hi = (pc >> 8) as u8;
                core.internal_stat.set_data((pc & 0xFF) as u8);
                push(core, ppu, cartridge, hub, apu, hi);
            }
            4 => {
                push(
                    core,
                    ppu,
                    cartridge,
                    hub,
                    apu,
                    core.internal_stat.get_data(),
                );
                core.internal_stat.set_interrupt(core.interrupt.nmi);

                core.internal_stat.set_address(if core.interrupt.nmi {
                    NMI_VECTOR
                } else {
                    IRQ_VECTOR
                });
            }
            5 => {
                let p =
                    (core.register.get_p() & !RegisterP::BREAK.bits()) | RegisterP::RESERVED.bits();
                push(core, ppu, cartridge, hub, apu, p);
            }
            6 => {
                core.register.set_i(true);
                core.internal_stat.set_data(core.memory.read(
                    core.internal_stat.get_address(),
                    ppu,
                    cartridge,
                    hub,
                    apu,
                    &mut core.interrupt,
                ));
                if core.internal_stat.get_interrupt() {
                    core.interrupt.nmi = false;
                }
            }
            7 => {
                let hi = u16::from(core.memory.read(
                    core.internal_stat.get_address() + 1,
                    ppu,
                    cartridge,
                    hub,
                    apu,
                    &mut core.interrupt,
                ));
                core.register
                    .set_pc((hi << 8) | u16::from(core.internal_stat.get_data()));
            }
            _ => {
                core.interrupt.executing = false;
                return exit_opcode(core);
            }
        }
        CpuStepStateEnum::Continue
    }
}

pub(crate) struct Reset;

impl CpuStepState for Reset {
    fn exec(
        core: &mut Core,
        ppu: &mut Ppu,
        cartridge: &mut dyn CpuCartridgeBus,
        hub: &mut dyn ControllerHub,
        apu: &mut Apu,
    ) -> CpuStepStateEnum {
        match core.internal_stat.get_step() {
            1 => {
                // dummy read
                read_dummy_current(core, ppu, cartridge, hub, apu);

                core.interrupt.irq_flag = IrqSource::empty();
                core.interrupt.irq_mask = IrqSource::ALL;
                core.interrupt.nmi = false;
            }
            2 => {
                // dummy read
                read_dummy_current(core, ppu, cartridge, hub, apu);
            }
            3 => {
                let sp = usize::from(core.register.get_sp());
                core.register.set_sp((sp.wrapping_sub(1) & 0xFF) as u8);
                let _ = core
                    .memory
                    .read(0x100 | sp, ppu, cartridge, hub, apu, &mut core.interrupt);
            }
            4 => {
                let sp = usize::from(core.register.get_sp());
                core.register.set_sp((sp.wrapping_sub(1) & 0xFF) as u8);
                let _ = core
                    .memory
                    .read(0x100 | sp, ppu, cartridge, hub, apu, &mut core.interrupt);
            }
            5 => {
                let sp = usize::from(core.register.get_sp());
                core.register.set_sp((sp.wrapping_sub(1) & 0xFF) as u8);
                let _ = core
                    .memory
                    .read(0x100 | sp, ppu, cartridge, hub, apu, &mut core.interrupt);
            }
            6 => {
                core.register.set_i(true);
                core.internal_stat.set_data(core.memory.read(
                    RESET_VECTOR,
                    ppu,
                    cartridge,
                    hub,
                    apu,
                    &mut core.interrupt,
                ));
            }
            7 => {
                let hi = u16::from(core.memory.read(
                    RESET_VECTOR + 1,
                    ppu,
                    cartridge,
                    hub,
                    apu,
                    &mut core.interrupt,
                ));
                core.register
                    .set_pc((hi << 8) | u16::from(core.internal_stat.get_data()));
                core.interrupt.executing = false;
            }
            _ => {
                return exit_opcode(core);
            }
        }
        CpuStepStateEnum::Continue
    }
}
